use crate::protocol::packets::TransferHeader;
use std::{net::SocketAddr, path::PathBuf, time::Instant};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub device_id: Uuid,
    pub device_name: String,
    pub addr: SocketAddr,
    pub transfer_port: u16,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferStatus {
    Pending,
    InProgress { bytes_transferred: u64, total: u64 },
    Completed,
    Rejected,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct IncomingTransfer {
    pub id: u64,
    pub header: TransferHeader,
    #[allow(dead_code)]
    pub from_addr: SocketAddr,
    pub status: TransferStatus,
    pub save_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
pub struct OutgoingTransfer {
    pub id: u64,
    pub filename: String,
    pub file_size: u64,
    pub target_device: String,
    pub status: TransferStatus,
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
    /// A human-readable status update for a single step within a transfer.
    StatusUpdate {
        #[allow(dead_code)]
        transfer_id: u64,
        message: Option<String>,
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
