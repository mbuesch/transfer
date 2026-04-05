use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const DISCOVERY_PORT: u16 = 42300;
pub const TRANSFER_PORT: u16 = 42301;
pub const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(4);

static IP_SUPPORT: OnceLock<IpSupport> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IpSupport {
    #[allow(dead_code)]
    V4,
    #[allow(dead_code)]
    V6,
    #[default]
    Both,
}

impl IpSupport {
    pub fn get() -> Self {
        IP_SUPPORT.get().copied().unwrap_or(IpSupport::default())
    }

    #[allow(dead_code)]
    pub fn set(&self) {
        if !cfg!(feature = "ipv4") {
            assert!(
                !matches!(self, IpSupport::V4 | IpSupport::Both),
                "IPv4 support is disabled at compile time"
            );
        }
        if !cfg!(feature = "ipv6") {
            assert!(
                !matches!(self, IpSupport::V6 | IpSupport::Both),
                "IPv6 support is disabled at compile time"
            );
        }
        let _ = IP_SUPPORT.set(*self);
    }

    pub fn ipv4() -> bool {
        matches!(Self::get(), IpSupport::V4 | IpSupport::Both) && cfg!(feature = "ipv4")
    }

    pub fn ipv6() -> bool {
        matches!(Self::get(), IpSupport::V6 | IpSupport::Both) && cfg!(feature = "ipv6")
    }
}

/// Network packet for device discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPacket {
    pub device_id: String,
    pub device_name: String,
    pub transfer_port: u16,
    pub checksum: [u8; 32],
}

/// Network packet header for file transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHeader {
    pub filename: String,
    pub file_size: u64,
    pub sender_name: String,
    pub header_checksum: [u8; 32],
    pub payload_checksum: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub device_id: String,
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
