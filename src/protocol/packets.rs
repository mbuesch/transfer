use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

pub const DISCOVERY_PORT: u16 = 42300;
pub const TRANSFER_PORT: u16 = 42301;
pub const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPacket {
    pub device_id: String,
    pub device_name: String,
    pub transfer_port: u16,
}

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub device_id: String,
    pub device_name: String,
    pub addr: SocketAddr,
    pub transfer_port: u16,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHeader {
    pub filename: String,
    pub file_size: u64,
    pub sender_name: String,
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
}

#[derive(Debug, Clone)]
pub struct OutgoingTransfer {
    pub id: u64,
    pub filename: String,
    pub file_size: u64,
    pub target_device: String,
    pub status: TransferStatus,
}
