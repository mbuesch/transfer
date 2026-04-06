use crate::fixedstr::FixedStr;
use anyhow::{self as ah, Context as _};
use crc_fast::{CrcAlgorithm::Crc64Nvme, Digest as CrcDigest};
use std::time::Duration;
use uuid::Uuid;

pub const DISCOVERY_PORT: u16 = 42300;
pub const TRANSFER_PORT: u16 = 42301;
pub const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(4);

pub fn checksum_new() -> CrcDigest {
    CrcDigest::new(Crc64Nvme)
}

/// Network packet for device discovery
#[derive(Debug, Clone, Default, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub struct DiscoveryPacket {
    pub device_id: (u64, u64),
    pub device_name: FixedStr<64>,
    pub transfer_port: u16,
    pub checksum: [u8; 8],
}

impl DiscoveryPacket {
    pub fn new(device_id: Uuid, device_name: &str, transfer_port: u16) -> Self {
        let device_id_int = device_id.as_u128();
        Self {
            device_id: (device_id_int as u64, (device_id_int >> 64) as u64),
            device_name: FixedStr::from_str_trunc(device_name),
            transfer_port,
            checksum: Self::compute_checksum(device_id, device_name.as_bytes(), transfer_port),
        }
    }

    pub const fn size() -> usize {
        96
    }

    pub fn device_id(&self) -> Uuid {
        Uuid::from_u128((self.device_id.0 as u128) | ((self.device_id.1 as u128) << 64))
    }

    pub fn serialize(&self) -> ah::Result<Vec<u8>> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self)?.into_vec();
        assert_eq!(
            bytes.len(),
            Self::size(),
            "DiscoveryPacket: Serialized size mismatch"
        );
        Ok(bytes)
    }

    pub fn deserialize(bytes: &[u8]) -> ah::Result<Self> {
        Ok(rkyv::from_bytes::<Self, rkyv::rancor::Error>(bytes)?)
    }

    fn compute_checksum(device_id: Uuid, device_name: &[u8], transfer_port: u16) -> [u8; 8] {
        let mut cs = checksum_new();
        cs.update(&device_id.as_u128().to_le_bytes());
        cs.update(device_name);
        cs.update(&transfer_port.to_le_bytes());
        cs.finalize().to_le_bytes()
    }

    pub fn verify_checksum(&self) -> bool {
        self.checksum
            == Self::compute_checksum(
                self.device_id(),
                self.device_name.as_bytes(),
                self.transfer_port,
            )
    }
}

/// Network packet header for file transfer
#[derive(Debug, Clone, Default, rkyv::Archive, rkyv::Deserialize, rkyv::Serialize)]
pub struct TransferHeader {
    pub filename: FixedStr<512>,
    pub file_size: u64,
    pub sender_name: FixedStr<64>,
    pub header_checksum: [u8; 8],
    pub payload_checksum: [u8; 8],
}

impl TransferHeader {
    pub fn new(
        filename: &str,
        file_size: u64,
        sender_name: &str,
        payload_checksum: [u8; 8],
    ) -> ah::Result<Self> {
        let filename_fixed = FixedStr::from_str(filename)
            .context("Filename is too long or contains invalid characters")?;
        let sender_name_fixed = FixedStr::from_str_trunc(sender_name);
        let header_checksum = Self::compute_header_checksum(
            filename_fixed.as_bytes(),
            file_size,
            sender_name_fixed.as_bytes(),
        );
        Ok(Self {
            filename: filename_fixed,
            file_size,
            sender_name: sender_name_fixed,
            header_checksum,
            payload_checksum,
        })
    }

    pub const fn size() -> usize {
        616
    }

    pub fn serialize(&self) -> ah::Result<Vec<u8>> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self)?.into_vec();
        assert_eq!(
            bytes.len(),
            Self::size(),
            "TransferHeader: Serialized size mismatch"
        );
        Ok(bytes)
    }

    pub fn deserialize(bytes: &[u8]) -> ah::Result<Self> {
        Ok(rkyv::from_bytes::<Self, rkyv::rancor::Error>(bytes)?)
    }

    fn compute_header_checksum(filename: &[u8], file_size: u64, sender_name: &[u8]) -> [u8; 8] {
        let mut cs = checksum_new();
        cs.update(filename);
        cs.update(&file_size.to_le_bytes());
        cs.update(sender_name);
        cs.finalize().to_le_bytes()
    }

    pub fn verify_header_checksum(&self) -> bool {
        self.header_checksum
            == Self::compute_header_checksum(
                self.filename.as_bytes(),
                self.file_size,
                self.sender_name.as_bytes(),
            )
    }
}
