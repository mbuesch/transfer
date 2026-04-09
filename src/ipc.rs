use crate::{crypto::RsaPublicKey, protocol::packets::TransferHeader};
use std::{net::SocketAddr, path::PathBuf, time::Instant};

/// The session password (optionally provided by the user at startup).
/// Shared as Arc so both the sender and receiver can access it.
pub type SessionPassword = std::sync::Arc<std::sync::Mutex<String>>;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub fingerprint: [u8; 32],
    pub device_name: String,
    pub addr: SocketAddr,
    pub transfer_port: u16,
    pub rsa_public_key: RsaPublicKey,
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
    /// The remote peer's stored key fingerprint does not match the presented one.
    /// The UI should prompt the user to accept or reject the connection.
    KeyMismatchWarning {
        transfer_id: u64,
        device_name: String,
        stored_fingerprint: String,
        presented_fingerprint: String,
        /// `true` if this is an incoming transfer, `false` if outgoing.
        is_incoming: bool,
    },
    /// The remote peer was encountered for the first time.
    /// The UI should prompt the user to accept or reject the new peer.
    NewPeerContact {
        transfer_id: u64,
        fingerprint: String,
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
    /// User accepted a key mismatch warning; trust the presented key and proceed.
    AcceptKeyChange {
        transfer_id: u64,
    },
    /// User rejected a key mismatch warning; abort the transfer.
    RejectKeyChange {
        transfer_id: u64,
    },
    /// User accepted a new (first-contact) peer; store their key and proceed.
    AcceptNewPeer {
        transfer_id: u64,
    },
    /// User rejected a new (first-contact) peer; abort the transfer.
    RejectNewPeer {
        transfer_id: u64,
    },
}
