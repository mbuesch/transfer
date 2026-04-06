use crate::protocol::packets::TransferHeader;
use std::{net::SocketAddr, time::Instant};
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
