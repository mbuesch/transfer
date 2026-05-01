use crate::{
    ipc::{IncomingTransfer, TransferCommand, TransferEvent, TransferStatus},
    protocol::packets::{TransferHeader, checksum_new},
};
use anyhow::{self as ah, Context as _, format_err as err};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
#[cfg(target_os = "android")]
use tempfile::tempdir;
use tempfile::tempfile;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
    task::JoinSet,
    time::{sleep, timeout},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[cfg(target_os = "android")]
use crate::android_interface::android_copy_folder_to_tree;

/// Transfer chunk size for reading/writing file data.
const CHUNK_SIZE: usize = 64 * 1024;
/// Compression level for zip packing.
const COMPRESSION_LEVEL: i64 = 1;
/// Minimum number of bytes between progress event emissions, to reduce UI re-render frequency.
const PROGRESS_REPORT_INTERVAL: u64 = 512 * 1024;
/// Timeout for receiving each chunk of data. If no data is received within this period, the transfer is aborted.
const CHUNK_RECEIVE_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for waiting for the header after a connection is established.
const HEADER_TIMEOUT: Duration = Duration::from_secs(15);
/// Overall timeout for waiting for a response from the receiver after sending the header.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a pending (awaiting user accept/reject) transfer is kept alive.
/// Slightly longer than RESPONSE_TIMEOUT so the sender's own timeout fires first.
const PENDING_TIMEOUT: Duration = Duration::from_secs(125);
/// Accept response.
const ACCEPT: [u8; 2] = [0x11, 0x11 ^ 0xFF];
/// Reject response.
const REJECT: [u8; 2] = [0x22, 0x22 ^ 0xFF];

/// Pack a file or directory into an anonymous temporary zip file.
/// Returns the file (seeked to the start) and the zip size in bytes.
///
/// This is a blocking function - call via `tokio::task::spawn_blocking`.
fn pack_to_zip(path: &Path) -> ah::Result<(File, u64)> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Zstd)
        .compression_level(Some(COMPRESSION_LEVEL));
    let zip_tmpfile = tempfile().context("Failed to create temporary ZIP file")?;
    let mut zip = ZipWriter::new(zip_tmpfile);
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    add_to_zip(&mut zip, options, base, path)?;
    let mut zip_tmpfile = zip.finish()?;
    let size = zip_tmpfile.stream_position()?;
    zip_tmpfile.seek(SeekFrom::Start(0))?;
    Ok((zip_tmpfile, size))
}

fn add_to_zip(
    zip: &mut ZipWriter<File>,
    options: SimpleFileOptions,
    base: &Path,
    entry: &Path,
) -> ah::Result<()> {
    if entry.is_dir() {
        for dir_entry in std::fs::read_dir(entry)? {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            let rel = path.strip_prefix(base)?;
            if path.is_dir() {
                zip.add_directory(format!("{}/", rel.to_string_lossy()), options)?;
            }
            add_to_zip(zip, options, base, &path)?;
        }
    } else {
        let rel = entry.strip_prefix(base)?;
        zip.start_file(rel.to_string_lossy(), options)?;
        let mut file = File::open(entry)?;
        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            zip.write_all(&buf[..n])?;
        }
    }
    Ok(())
}

/// Extracts a zip archive from a temporary file into `save_path`.
/// Skips entries with unsafe (path-traversal) names.
///
/// This is a blocking function - call via `tokio::task::spawn_blocking`.
fn extract_zip(file: File, save_path: &Path) -> ah::Result<()> {
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(rel_path) = file.enclosed_name() else {
            log::warn!("Skipping zip entry with unsafe path: {}", file.name());
            continue;
        };
        let out_path = save_path.join(rel_path);
        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let out_path = find_available_path(&out_path)?;
            let mut out_file = File::create(&out_path)?;
            let mut buf = vec![0u8; CHUNK_SIZE];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                out_file.write_all(&buf[..n])?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "android")]
fn extract_zip_to_android_tree(file: File, tree_uri: &str) -> ah::Result<()> {
    let temp_dir = tempdir().context("Failed to create temporary extraction directory")?;
    let save_path = temp_dir.path();
    extract_zip(file, save_path)?;
    android_copy_folder_to_tree(tree_uri, save_path)
        .context("Failed to copy extracted files to selected Android folder")
}

/// Creates a dual-stack TCP listener that accepts both IPv4 and IPv6 connections.
fn create_tcp_listener(port: u16) -> ah::Result<TcpListener> {
    let sock = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    sock.set_only_v6(false)?; // Accept IPv4-mapped addresses as well
    sock.set_reuse_address(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into())?;
    sock.listen(128)?;
    let std_listener: std::net::TcpListener = sock.into();
    let listener = TcpListener::from_std(std_listener)?;
    Ok(listener)
}

/// Helper to emit a `StatusUpdate` event.
fn send_status(
    event_tx: &mpsc::UnboundedSender<TransferEvent>,
    transfer_id: u64,
    msg: Option<&str>,
) {
    let _ = event_tx.send(TransferEvent::StatusUpdate {
        transfer_id,
        message: msg.map(|s| s.to_string()),
    });
}

struct PendingIncoming {
    #[allow(dead_code)]
    transfer_id: u64,
    header: TransferHeader,
    stream: TcpStream,
}

pub async fn run_transfer_server(
    port: u16,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
    mut cmd_rx: mpsc::UnboundedReceiver<TransferCommand>,
) {
    let listener = match create_tcp_listener(port) {
        Ok(l) => {
            log::info!("Transfer server listening on [::]:{port} (dual-stack)");
            l
        }
        Err(e) => {
            log::warn!("IPv6 dual-stack unavailable ({e}), falling back to IPv4");
            match TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await {
                Ok(l) => l,
                Err(e) => {
                    log::error!("Failed to bind transfer server on port {port}: {e}");
                    return;
                }
            }
        }
    };

    let pending = Arc::new(Mutex::new(HashMap::new()));

    let pending_clone = Arc::clone(&pending);
    let event_tx_clone = event_tx.clone();

    // Spawn acceptor task
    let accept_handle = tokio::spawn(async move {
        // JoinSet owns all per-connection tasks. When this acceptor task is
        // aborted (via accept_handle.abort()), the JoinSet is dropped, which
        // aborts every tracked task in it.
        let mut join_set: JoinSet<()> = JoinSet::new();
        let mut next_id: u64 = 1;
        loop {
            let (stream, addr) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Accept error: {e}");
                    continue;
                }
            };
            let event_tx = event_tx_clone.clone();
            let pending = Arc::clone(&pending_clone);

            let transfer_id = next_id;
            next_id += 1;

            // Reap completed tasks to avoid unbounded JoinSet growth.
            while join_set.try_join_next().is_some() {}

            join_set.spawn(async move {
                if let Err(e) =
                    handle_incoming_connection(stream, addr, transfer_id, event_tx, pending).await
                {
                    log::error!("Incoming connection handling error: {e}");
                }
            });
        }
    });

    // Process commands
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            TransferCommand::AcceptTransfer {
                transfer_id,
                save_path,
            } => {
                let mut map = pending.lock().await;
                if let Some(incoming) = map.remove(&transfer_id) {
                    let event_tx = event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = receive_file(
                            incoming.stream,
                            incoming.header,
                            transfer_id,
                            save_path,
                            event_tx,
                        )
                        .await
                        {
                            log::warn!(
                                "Error during file reception for transfer {transfer_id}: {e}"
                            );
                        }
                    });
                }
            }
            TransferCommand::RejectTransfer { transfer_id } => {
                let mut map = pending.lock().await;
                if let Some(incoming) = map.remove(&transfer_id) {
                    // Send rejection and close
                    let mut stream = incoming.stream;
                    let _ = stream.write_all(&REJECT).await;
                    let _ = stream.shutdown().await;
                    let _ = event_tx.send(TransferEvent::Rejected { transfer_id });
                }
            }
        }
    }

    accept_handle.abort();
}

async fn handle_incoming_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    transfer_id: u64,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
    pending: Arc<Mutex<HashMap<u64, PendingIncoming>>>,
) -> ah::Result<()> {
    log::debug!("Read header for incoming transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Reading header..."));
    let mut header_buf = vec![0u8; TransferHeader::size()];
    match timeout(HEADER_TIMEOUT, stream.read_exact(&mut header_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(err!("Timeout while reading header")),
    }
    let header = TransferHeader::deserialize(&header_buf)?;
    let header_filename = header.filename.as_str().context("Decode filename failed")?;
    let header_sender_name = header
        .sender_name
        .as_str()
        .context("Decode sender name failed")?;

    log::debug!("Verifying header checksum for incoming transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Verifying header..."));
    if !header.verify_header_checksum() {
        return Err(err!("Header checksum mismatch"));
    }

    log::info!(
        "Incoming transfer from {}: {} ({} bytes)",
        header_sender_name,
        header_filename,
        header.file_size
    );

    let incoming = IncomingTransfer {
        id: transfer_id,
        header: header.clone(),
        from_addr: addr,
        status: TransferStatus::Pending,
        save_path: None,
    };

    // Insert into the pending map BEFORE sending the IncomingRequest event.
    // This prevents a race where auto-accept immediately sends AcceptTransfer
    // and the command loop finds an empty map.
    {
        let mut map = pending.lock().await;
        map.insert(
            transfer_id,
            PendingIncoming {
                transfer_id,
                header,
                stream,
            },
        );
    }

    let _ = event_tx.send(TransferEvent::IncomingRequest(Box::new(incoming)));

    // Spawn an expiry task: if the transfer is neither accepted nor rejected
    // within PENDING_TIMEOUT, close the dead connection and emit a failure
    // event to the UI to avoid a permanent FD leak.
    tokio::spawn({
        let pending = Arc::clone(&pending);
        let event_tx = event_tx.clone();
        async move {
            sleep(PENDING_TIMEOUT).await;
            let mut map = pending.lock().await;
            if map.remove(&transfer_id).is_some() {
                // The TcpStream inside PendingIncoming is dropped here, closing the OS socket.
                let _ = event_tx.send(TransferEvent::Failed {
                    transfer_id,
                    error: "Transfer timed out waiting for user response".to_string(),
                });
            }
        }
    });

    Ok(())
}

/// Find a non-colliding filename by appending numbers if the file already exists.
/// If the original file doesn't exist, returns it as-is.
/// Otherwise, tries filename (1), filename (2), etc. until finding an available name.
/// Returns an error if unable to find an available name.
fn find_available_path(file_path: &PathBuf) -> ah::Result<PathBuf> {
    if std::fs::metadata(file_path).is_err() {
        // File doesn't exist, safe to use original path
        return Ok(file_path.clone());
    }

    // File exists, need to find an alternative
    let parent = file_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let file_name = file_path
        .file_name()
        .ok_or_else(|| err!("Invalid file path: {}", file_path.display()))?;
    let file_name_str = file_name.to_string_lossy();

    // Split filename into name and extension
    let (base_name, extension) = if let Some(dot_pos) = file_name_str.rfind('.') {
        let (name, ext) = file_name_str.split_at(dot_pos);
        (name.to_string(), ext.to_string())
    } else {
        (file_name_str.to_string(), String::new())
    };

    // Try appending numbers until we find an available name
    for i in 1..=10000 {
        let new_name = format!("{base_name} ({i}){extension}");
        let new_path = parent.join(&new_name);

        if std::fs::metadata(&new_path).is_err() {
            return Ok(new_path);
        }
    }

    // Unable to find an available filename
    Err(err!(
        "Could not find available filename for {file_name_str}. File already exists."
    ))
}

async fn receive_file(
    mut stream: TcpStream,
    header: TransferHeader,
    transfer_id: u64,
    save_path: PathBuf,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
) -> ah::Result<()> {
    log::debug!("Accepting transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Accepting..."));
    if let Err(e) = stream.write_all(&ACCEPT).await {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: format!("Failed to send accept: {e}"),
        });
        return Err(e.into());
    }

    log::debug!("Starting to receive zip data for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Receiving..."));
    let mut received: u64 = 0;
    let mut last_reported: u64 = 0;
    let total = header.file_size;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let zip_tmpfile = tempfile().context("Failed to create temporary ZIP file")?;
    let mut zip_file = tokio::fs::File::from_std(zip_tmpfile);
    let mut cs = checksum_new();
    loop {
        if received >= total {
            break;
        }

        let to_read = std::cmp::min(CHUNK_SIZE as u64, total - received) as usize;
        match timeout(CHUNK_RECEIVE_TIMEOUT, stream.read(&mut buf[..to_read])).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                cs.update(&buf[..n]);
                zip_file.write_all(&buf[..n]).await?;
                received += n as u64;
                if received - last_reported >= PROGRESS_REPORT_INTERVAL || received >= total {
                    last_reported = received;
                    let _ = event_tx.send(TransferEvent::Progress {
                        transfer_id,
                        bytes_transferred: received,
                        total,
                    });
                }
            }
            Ok(Err(e)) => {
                let _ = event_tx.send(TransferEvent::Failed {
                    transfer_id,
                    error: format!("Read error: {e}"),
                });
                return Err(e.into());
            }
            Err(_elapsed) => {
                let _ = event_tx.send(TransferEvent::Failed {
                    transfer_id,
                    error: "Transfer timed out: no data received".to_string(),
                });
                return Err(err!("Transfer timed out: no data received"));
            }
        }
    }

    if received == 0 {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: "No data received".to_string(),
        });
        return Err(err!("No data received"));
    }

    if received != total {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: format!("Incomplete transfer: received {received} of {total} bytes"),
        });
        return Err(err!(
            "Incomplete transfer: received {received} of {total} bytes"
        ));
    }

    log::debug!("Verifying payload checksum for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Verifying checksum..."));
    let computed = cs.finalize().to_le_bytes();
    if computed != header.payload_checksum {
        log::error!("Transfer {transfer_id}: payload checksum mismatch.");
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: "Payload checksum mismatch - data corrupted in transit".to_string(),
        });
        return Err(err!(
            "Payload checksum mismatch - data corrupted in transit"
        ));
    }

    log::debug!("Extracting zip for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Extracting..."));
    zip_file.seek(SeekFrom::Start(0)).await?;
    let zip_std_file = zip_file.into_std().await;
    let save_path_clone = save_path.clone();
    let extraction_result = tokio::task::spawn_blocking(move || {
        if let Some(path_str) = save_path_clone.to_str() {
            if path_str.starts_with("content://") {
                #[cfg(target_os = "android")]
                {
                    extract_zip_to_android_tree(zip_std_file, path_str)
                }
                #[cfg(not(target_os = "android"))]
                {
                    extract_zip(zip_std_file, &save_path_clone)
                }
            } else {
                extract_zip(zip_std_file, &save_path_clone)
            }
        } else {
            extract_zip(zip_std_file, &save_path_clone)
        }
    })
    .await
    .context("Extraction task panicked")?;
    if let Err(e) = extraction_result {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: format!("Extraction failed: {e}"),
        });
        return Err(e);
    }

    let _ = event_tx.send(TransferEvent::Completed {
        transfer_id,
        save_path: Some(save_path.clone()),
    });
    log::info!(
        "Transfer {transfer_id} completed, extracted to {}",
        save_path.display()
    );

    send_status(&event_tx, transfer_id, None);

    Ok(())
}

pub async fn send_path(
    target_addr: SocketAddr,
    path: PathBuf,
    sender_name: String,
    transfer_id: u64,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
) -> ah::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    log::debug!("Packing {:?} for transfer {transfer_id}", path.display());
    send_status(&event_tx, transfer_id, Some("Packing..."));
    let path_clone = path.clone();
    let (zip_std_file, zip_size) =
        match tokio::task::spawn_blocking(move || pack_to_zip(&path_clone))
            .await
            .context("Pack task panicked")?
        {
            Ok(v) => v,
            Err(e) => {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: format!("Failed to pack: {e}"),
                });
                return Err(e);
            }
        };
    if zip_size == 0 {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: "Nothing to send".to_string(),
        });
        return Err(err!("Nothing to send"));
    }

    log::debug!("Calculating checksum for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Calculating checksum..."));
    let (zip_std_file, payload_checksum) = tokio::task::spawn_blocking(move || -> ah::Result<_> {
        let mut zip_std_file = zip_std_file;
        let mut cs = checksum_new();
        let mut ck_buf = vec![0u8; CHUNK_SIZE];
        loop {
            let n = zip_std_file.read(&mut ck_buf)?;
            if n == 0 {
                break;
            }
            cs.update(&ck_buf[..n]);
        }
        let checksum = cs.finalize().to_le_bytes();
        zip_std_file.seek(SeekFrom::Start(0))?;
        Ok((zip_std_file, checksum))
    })
    .await
    .context("Checksum task panicked")??;
    let mut zip_file = tokio::fs::File::from_std(zip_std_file);

    log::debug!(
        "Connecting to {} for transfer {transfer_id}...",
        target_addr
    );
    send_status(&event_tx, transfer_id, Some("Connecting..."));
    let mut stream = match TcpStream::connect(target_addr).await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Connection failed: {e}"),
            });
            return Err(e.into());
        }
    };

    log::debug!("Sending header for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Sending header..."));
    let header = TransferHeader::new(&name, zip_size, &sender_name, payload_checksum)?;
    let header_bytes = match header.serialize() {
        Ok(b) => b,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Serialization error: {e}"),
            });
            return Err(e);
        }
    };
    if let Err(e) = stream.write_all(&header_bytes).await {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: format!("Failed to send header: {e}"),
        });
        return Err(e.into());
    }

    log::debug!("Waiting for accept/reject response for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Waiting for response..."));
    let mut response_buf = [0u8; 2];
    match timeout(RESPONSE_TIMEOUT, stream.read_exact(&mut response_buf)).await {
        Ok(Ok(_)) => match response_buf {
            ACCEPT => (),
            REJECT => {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: "Transfer rejected by receiver".to_string(),
                });
                return Err(err!("Transfer rejected by receiver"));
            }
            _ => {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: "Invalid response from receiver".to_string(),
                });
                return Err(err!("Invalid response from receiver"));
            }
        },
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: "Connection closed before response".to_string(),
            });
            return Err(err!("Connection closed before response"));
        }
        Ok(Err(e)) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Error reading response: {e}"),
            });
            return Err(e.into());
        }
        Err(_) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: "Timed out waiting for response".to_string(),
            });
            return Err(err!("Timed out waiting for response"));
        }
    }

    log::debug!("Starting zip stream for transfer {transfer_id}...");
    send_status(&event_tx, transfer_id, Some("Sending..."));
    let total = zip_size;
    let mut sent: u64 = 0;
    let mut last_reported: u64 = 0;
    let mut send_buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = match zip_file.read(&mut send_buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: format!("Send error: {e}"),
                });
                return Err(e.into());
            }
        };
        if let Err(e) = stream.write_all(&send_buf[..n]).await {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Send error: {e}"),
            });
            return Err(e.into());
        }
        sent += n as u64;
        if sent - last_reported >= PROGRESS_REPORT_INTERVAL || sent >= total {
            last_reported = sent;
            let _ = event_tx.send(TransferEvent::SendProgress {
                transfer_id,
                bytes_sent: sent,
                total,
            });
        }
    }

    let _ = stream.shutdown().await;
    send_status(&event_tx, transfer_id, None);
    let _ = event_tx.send(TransferEvent::SendCompleted { transfer_id });
    log::info!("Transfer {transfer_id} completed: {}", path.display());

    Ok(())
}
