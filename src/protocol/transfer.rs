use crate::protocol::packets::{IncomingTransfer, TransferHeader, TransferStatus};
use dioxus::prelude::*;
use sha3::{Digest, Sha3_256};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc},
};

const HEADER_SIZE_LIMIT: usize = 64 * 1024;
const CHUNK_SIZE: usize = 64 * 1024;

fn compute_header_checksum(filename: &str, file_size: u64, sender_name: &str) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(filename.as_bytes());
    hasher.update(file_size.to_le_bytes());
    hasher.update(sender_name.as_bytes());
    hasher.finalize().into()
}

/// Creates a dual-stack TCP listener that accepts both IPv4 and IPv6 connections.
/// Falls back to IPv4-only when IPv6 is unavailable.
fn create_tcp_listener(port: u16) -> std::io::Result<std::net::TcpListener> {
    let sock = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    sock.set_only_v6(false)?; // Accept IPv4-mapped addresses as well
    sock.set_reuse_address(true)?;
    sock.set_nonblocking(true)?;
    sock.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into())?;
    sock.listen(128)?;
    Ok(sock.into())
}

/// Events from the transfer server to the UI
#[derive(Debug, Clone)]
pub enum TransferEvent {
    IncomingRequest(IncomingTransfer),
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
    let listener = match create_tcp_listener(port).and_then(TcpListener::from_std) {
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
    let accept_handle = spawn(async move {
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

            spawn(async move {
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
                    spawn(async move {
                        receive_file(
                            incoming.stream,
                            incoming.header,
                            transfer_id,
                            save_path,
                            event_tx,
                        )
                        .await;
                    });
                }
            }
            TransferCommand::RejectTransfer { transfer_id } => {
                let mut map = pending.lock().await;
                if let Some(incoming) = map.remove(&transfer_id) {
                    // Send rejection and close
                    let mut stream = incoming.stream;
                    let _ = stream.write_all(b"REJECT\n").await;
                    let _ = stream.shutdown().await;
                    let _ = event_tx.send(TransferEvent::Rejected { transfer_id });
                }
            }
        }
    }

    accept_handle.cancel();
}

async fn handle_incoming_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    transfer_id: u64,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
    pending: Arc<Mutex<HashMap<u64, PendingIncoming>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Read header length (4 bytes, big-endian)
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let header_len = u32::from_be_bytes(len_buf) as usize;

    if header_len > HEADER_SIZE_LIMIT {
        return Err("Header too large".into());
    }

    // Read header JSON
    let mut header_buf = vec![0u8; header_len];
    stream.read_exact(&mut header_buf).await?;
    let header: TransferHeader = serde_json::from_slice(&header_buf)?;

    // Verify header checksum before doing anything with the transfer.
    let expected_header_checksum =
        compute_header_checksum(&header.filename, header.file_size, &header.sender_name);
    if expected_header_checksum != header.header_checksum {
        return Err("Header checksum mismatch".into());
    }

    log::info!(
        "Incoming transfer from {}: {} ({} bytes)",
        header.sender_name,
        header.filename,
        header.file_size
    );

    let incoming = IncomingTransfer {
        id: transfer_id,
        header: header.clone(),
        from_addr: addr,
        status: TransferStatus::Pending,
        save_path: None,
    };

    let _ = event_tx.send(TransferEvent::IncomingRequest(incoming));

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
async fn find_available_path(file_path: &PathBuf) -> Result<PathBuf, String> {
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
        .ok_or_else(|| format!("Invalid file path: {}", file_path.display()))?;
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
    Err(format!(
        "Could not find available filename for {file_name_str}. File already exists."
    ))
}

async fn receive_file(
    mut stream: TcpStream,
    header: TransferHeader,
    transfer_id: u64,
    save_path: PathBuf,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
) {
    let file_path = save_path.join(&header.filename);

    // Check if file exists and find an available alternative name if needed
    let file_path = match find_available_path(&file_path).await {
        Ok(path) => path,
        Err(e) => {
            log::error!("Failed to find available path: {e}");
            let _ = stream.write_all(b"REJECT\n").await;
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id,
                error: e,
            });
            return;
        }
    };

    let mut file = match tokio::fs::File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to create save file {file_path:?}: {e}");
            let _ = stream.write_all(b"REJECT\n").await;
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id,
                error: format!("Failed to create file: {e}"),
            });
            return;
        }
    };

    // Send acceptance only after confirming we can actually save the file.
    if let Err(e) = stream.write_all(b"ACCEPT\n").await {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: format!("Failed to send accept: {e}"),
        });
        return;
    }

    let mut received: u64 = 0;
    let total = header.file_size;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut hasher = Sha3_256::new();

    loop {
        if received >= total {
            break;
        }

        let to_read = std::cmp::min(CHUNK_SIZE as u64, total - received) as usize;
        match stream.read(&mut buf[..to_read]).await {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                if let Err(e) = file.write_all(&buf[..n]).await {
                    let _ = event_tx.send(TransferEvent::Failed {
                        transfer_id,
                        error: format!("Write error: {e}"),
                    });
                    return;
                }
                received += n as u64;
                let _ = event_tx.send(TransferEvent::Progress {
                    transfer_id,
                    bytes_transferred: received,
                    total,
                });
            }
            Err(e) => {
                let _ = event_tx.send(TransferEvent::Failed {
                    transfer_id,
                    error: format!("Read error: {e}"),
                });
                return;
            }
        }
    }

    if received == 0 {
        let _ = event_tx.send(TransferEvent::Failed {
            transfer_id,
            error: "No data received".to_string(),
        });
    } else if received == total {
        let computed: [u8; 32] = hasher.finalize().into();
        if computed != header.payload_checksum {
            log::error!("Transfer {transfer_id}: payload checksum mismatch.");
            drop(file);
            let _ = tokio::fs::remove_file(&file_path).await;
            let _ = event_tx.send(TransferEvent::Failed {
                transfer_id,
                error: "Payload checksum mismatch - file corrupted in transit".to_string(),
            });
            return;
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
}

pub async fn send_file(
    target_addr: SocketAddr,
    file_path: PathBuf,
    sender_name: String,
    transfer_id: u64,
    event_tx: mpsc::UnboundedSender<TransferEvent>,
) {
    let metadata = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Cannot read file metadata: {e}"),
            });
            return;
        }
    };

    let mut file = match tokio::fs::File::open(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Cannot open file: {e}"),
            });
            return;
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
        return;
    }

    let payload_checksum = {
        let mut hasher = Sha3_256::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            match file.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(e) => {
                    let _ = event_tx.send(TransferEvent::SendFailed {
                        transfer_id,
                        error: format!("File read error computing payload checksum: {e}"),
                    });
                    return;
                }
            }
        }
        hasher.finalize().into()
    };

    if let Err(e) = file.seek(std::io::SeekFrom::Start(0)).await {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: format!("Failed to seek file after computing payload checksum: {e}"),
        });
        return;
    }

    let header = TransferHeader {
        filename: filename.clone(),
        file_size,
        sender_name: sender_name.clone(),
        header_checksum: compute_header_checksum(&filename, file_size, &sender_name),
        payload_checksum,
    };

    let mut stream = match TcpStream::connect(target_addr).await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Connection failed: {e}"),
            });
            return;
        }
    };

    // Send header
    let header_bytes = match serde_json::to_vec(&header) {
        Ok(b) => b,
        Err(e) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Serialization error: {e}"),
            });
            return;
        }
    };

    let len_bytes = (header_bytes.len() as u32).to_be_bytes();
    if let Err(e) = stream.write_all(&len_bytes).await {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: format!("Failed to send header length: {e}"),
        });
        return;
    }
    if let Err(e) = stream.write_all(&header_bytes).await {
        let _ = event_tx.send(TransferEvent::SendFailed {
            transfer_id,
            error: format!("Failed to send header: {e}"),
        });
        return;
    }

    // Wait for accept/reject response
    let mut response_buf = [0u8; 64];
    match tokio::time::timeout(
        std::time::Duration::from_secs(120),
        stream.read(&mut response_buf),
    )
    .await
    {
        Ok(Ok(n)) if n > 0 => {
            let response = String::from_utf8_lossy(&response_buf[..n]);
            if response.trim() == "REJECT" {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: "Transfer rejected by receiver".to_string(),
                });
                return;
            }
            // ACCEPT - proceed
        }
        Ok(Ok(_)) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: "Connection closed before response".to_string(),
            });
            return;
        }
        Ok(Err(e)) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: format!("Error reading response: {e}"),
            });
            return;
        }
        Err(_) => {
            let _ = event_tx.send(TransferEvent::SendFailed {
                transfer_id,
                error: "Timed out waiting for response".to_string(),
            });
            return;
        }
    }

    // Stream file data.
    let total = metadata.len();
    let mut sent: u64 = 0;
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        match file.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = stream.write_all(&buf[..n]).await {
                    let _ = event_tx.send(TransferEvent::SendFailed {
                        transfer_id,
                        error: format!("Send error: {e}"),
                    });
                    return;
                }
                sent += n as u64;
                let _ = event_tx.send(TransferEvent::SendProgress {
                    transfer_id,
                    bytes_sent: sent,
                    total,
                });
            }
            Err(e) => {
                let _ = event_tx.send(TransferEvent::SendFailed {
                    transfer_id,
                    error: format!("File read error: {e}"),
                });
                return;
            }
        }
    }

    let _ = stream.shutdown().await;
    let _ = event_tx.send(TransferEvent::SendCompleted { transfer_id });
    log::info!("File sent successfully: transfer {transfer_id}");
}
