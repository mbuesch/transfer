use crate::{
    ipc::{IncomingTransfer, TransferStatus},
    protocol::packets::{TransferHeader, checksum_new},
};
use anyhow::{self as ah, Context as _, format_err as err};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
    time::timeout,
};

/// Transfer chunk size for reading/writing file data.
const CHUNK_SIZE: usize = 64 * 1024;
/// Minimum number of bytes between progress event emissions, to reduce UI re-render frequency.
const PROGRESS_REPORT_INTERVAL: u64 = 512 * 1024;
/// Timeout for receiving each chunk of data. If no data is received within this period, the transfer is aborted.
const CHUNK_RECEIVE_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for waiting for the header after a connection is established.
const HEADER_TIMEOUT: Duration = Duration::from_secs(15);
/// Overall timeout for waiting for a response from the receiver after sending the header.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
/// Accept response.
const ACCEPT: [u8; 2] = [0x11, 0x11 ^ 0xFF];
/// Reject response.
const REJECT: [u8; 2] = [0x22, 0x22 ^ 0xFF];

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

/// Events from the transfer server to the UI
#[derive(Debug, Clone)]
pub enum TransferEvent {
    IncomingRequest(Box<IncomingTransfer>),
    Progress {
        transfer_id: u64,
        bytes_transferred: u64,
        total: u64,
    },
    Completed {
        transfer_id: u64,
        save_path: Option<PathBuf>,
    },
    Rejected {
        transfer_id: u64,
    },
    Failed {
        transfer_id: u64,
        error: String,
    },
    SendProgress {
        transfer_id: u64,
        bytes_sent: u64,
        total: u64,
    },
    SendCompleted {
        transfer_id: u64,
    },
    SendFailed {
        transfer_id: u64,
        error: String,
    },
}

/// Commands from the UI to the transfer server
#[derive(Debug)]
pub enum TransferCommand {
    AcceptTransfer {
        transfer_id: u64,
        save_path: PathBuf,
    },
    RejectTransfer {
        transfer_id: u64,
    },
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

            tokio::spawn(async move {
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

    let _ = event_tx.send(TransferEvent::IncomingRequest(Box::new(incoming)));

    // Store pending connection
    let mut map = pending.lock().await;
    map.insert(
        transfer_id,
        PendingIncoming {
            transfer_id,
            header,
            stream,
        },
    );

    Ok(())
}

/// Find a non-colliding filename by appending numbers if the file already exists.
/// If the original file doesn't exist, returns it as-is.
/// Otherwise, tries filename (1), filename (2), etc. until finding an available name.
/// Returns an error if unable to find an available name.
async fn find_available_path(file_path: &PathBuf) -> ah::Result<PathBuf> {
    if tokio::fs::metadata(file_path).await.is_err() {
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

        if tokio::fs::metadata(&new_path).await.is_err() {
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
    log::debug!("Preparing to receive file for transfer {transfer_id}...");
    let header_filename = header.filename.as_str().context("Decode filename failed")?;

    // Check if file exists and find an available alternative name if needed
    let file_path = save_path.join(header_filename);
    let file_path = match find_available_path(&file_path).await {
        Ok(path) => path,
        Err(e) => {
            log::error!("Failed to find available path: {e}");
            let _ = stream.write_all(&REJECT).await;
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id,
                error: e.to_string(),
            });
            return Err(e);
        }
    };

    log::debug!(
        "Opening target file {:?} for transfer {transfer_id}",
        file_path.display()
    );
    let mut file = match tokio::fs::File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to create save file {file_path:?}: {e}");
            let _ = stream.write_all(&REJECT).await;
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id,
                error: format!("Failed to create file: {e}"),
            });
            return Err(e.into());
        }
    };

    log::debug!("Accepting transfer {transfer_id} and ready to receive data...");
    if let Err(e) = stream.write_all(&ACCEPT).await {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: format!("Failed to send accept: {e}"),
        });
        return Err(e.into());
    }

    log::debug!("Starting to receive file data for transfer {transfer_id}...");
    let mut received: u64 = 0;
    let mut last_reported: u64 = 0;
    let total = header.file_size;
    let mut buf = vec![0u8; CHUNK_SIZE];
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
                if let Err(e) = file.write_all(&buf[..n]).await {
                    let _ = event_tx.send(TransferEvent::Failed {
                        transfer_id,
                        error: format!("Write error: {e}"),
                    });
                    return Err(e.into());
                }
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
    } else if received == total {
        log::debug!("Verifying payload checksum for transfer {transfer_id}...");
        let computed = cs.finalize().to_le_bytes();
        if computed != header.payload_checksum {
            log::error!("Transfer {transfer_id}: payload checksum mismatch.");
            drop(file);
            let _ = tokio::fs::remove_file(&file_path).await;
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id,
                error: "Payload checksum mismatch - file corrupted in transit".to_string(),
            });
            return Err(err!(
                "Payload checksum mismatch - file corrupted in transit"
            ));
        }
        let _ = event_tx.send(TransferEvent::Completed {
            transfer_id,
            save_path: Some(file_path.clone()),
        });
        log::info!(
            "Transfer {transfer_id} completed (checksum OK): {}",
            file_path.display()
        );
    } else {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: format!("Incomplete transfer: received {received} of {total} bytes"),
        });
    }

    Ok(())
}

pub async fn send_file(
    target_addr: SocketAddr,
    file_path: PathBuf,
    sender_name: String,
    transfer_id: u64,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
) -> ah::Result<()> {
    log::debug!(
        "Opening file {:?} for transfer {transfer_id}",
        file_path.display()
    );
    let metadata = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Cannot read file metadata: {e}"),
            });
            return Err(e.into());
        }
    };
    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Cannot open file: {e}"),
            });
            return Err(e.into());
        }
    };
    let filename = file_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let file_size = metadata.len();
    if file_size == 0 {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: "Cannot send empty file".to_string(),
        });
        return Err(err!("Cannot send empty file"));
    }

    log::debug!("Calculating payload checksum for transfer {transfer_id}...");
    let payload_checksum = {
        let mut cs = checksum_new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            match file.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => cs.update(&buf[..n]),
                Err(e) => {
                    let _ = event_tx.send(TransferEvent::SendFailed {
                        transfer_id,
                        error: format!("File read error computing payload checksum: {e}"),
                    });
                    return Err(e.into());
                }
            }
        }
        cs.finalize().to_le_bytes()
    };
    if let Err(e) = file.seek(std::io::SeekFrom::Start(0)).await {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: format!("Failed to seek file after computing payload checksum: {e}"),
        });
        return Err(e.into());
    }

    log::debug!(
        "Connecting to {} for transfer {transfer_id}...",
        target_addr
    );
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
    let header = TransferHeader::new(&filename, file_size, &sender_name, payload_checksum)?;
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
    let mut response_buf = [0u8; 2];
    match timeout(RESPONSE_TIMEOUT, stream.read_exact(&mut response_buf)).await {
        Ok(Ok(0)) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: "Connection closed before response".to_string(),
            });
            return Err(err!("Connection closed before response"));
        }
        Ok(Ok(_)) => {
            match response_buf {
                ACCEPT => (), // Proceed with transfer
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
            }
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

    log::debug!("Starting file transfer for transfer {transfer_id}...");
    let total = metadata.len();
    let mut sent: u64 = 0;
    let mut last_reported: u64 = 0;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        match file.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                match stream.write_all(&buf[..n]).await {
                    Ok(()) => {}
                    Err(e) => {
                        let _ = event_tx.send(TransferEvent::SendFailed {
                            transfer_id,
                            error: format!("Send error: {e}"),
                        });
                        return Err(e.into());
                    }
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
            Err(e) => {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: format!("File read error: {e}"),
                });
                return Err(e.into());
            }
        }
    }

    let _ = stream.shutdown().await;
    let _ = event_tx.send(TransferEvent::SendCompleted { transfer_id });
    log::info!("File sent successfully: transfer {transfer_id}");

    Ok(())
}
