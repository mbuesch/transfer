use crate::{
    crypto::{
        NONCE_LEN, RsaPrivateKey, RsaPublicKey, public_key_from_der, public_key_to_der, rsa_sign,
        rsa_verify,
    },
    fixedstr::FixedStr,
};
use anyhow::{self as ah, Context as _, format_err as err};
use crc_fast::{CrcAlgorithm::Crc64Nvme, Digest as CrcDigest};
use std::time::Duration;

pub const DISCOVERY_PORT: u16 = 42300;
pub const TRANSFER_PORT: u16 = 42301;
pub const BROADCAST_INTERVAL: Duration = Duration::from_secs(1);
pub const DEVICE_TIMEOUT: Duration = Duration::from_secs(4);

pub fn checksum_new() -> CrcDigest {
    CrcDigest::new(Crc64Nvme)
}

/// Exact DER byte size of an RSA-4096 SubjectPublicKeyInfo (PKCS#8 public key).
///
/// Derivation (all sizes in bytes):
///
/// ```text
/// Modulus INTEGER:      RSA-4096 always has a 512-byte modulus with the high bit set,
///                       so DER prepends a 0x00 sign byte (513-byte value).
///                       TLV: 02 82 02 01 + 513 = 517 bytes.
/// Exponent INTEGER:     e = 65537 = 0x010001 (3 bytes of value).
///                       TLV: 02 03 01 00 01 = 5 bytes.
/// RSAPublicKey SEQ:     517 + 5 = 522 bytes content.  TLV: 30 82 02 0A + 522 = 526 bytes.
/// BIT STRING:           0x00 unused-bits byte + 526 = 527 bytes content.
///                       TLV: 03 82 02 0F + 527 = 531 bytes.
/// AlgorithmIdentifier:  OID rsaEncryption (11 bytes) + NULL (2 bytes) = 13 bytes content.
///                       TLV: 30 0D + 13 = 15 bytes.
/// SubjectPublicKeyInfo: 15 + 531 = 546 bytes content.  TLV: 30 82 02 22 + 546 = 550 bytes.
/// ```
///
/// The value is deterministic for all RSA-4096 keys with e = 65537.
const RSA_PUB_DER_MAX: usize = 550;

/// RSA modulus / signature size in bytes.
pub const RSA_SIGNATURE_SIZE: usize = 512;

/// SHA3-256 fingerprint size in bytes (used to identify a peer in handshake packets).
pub const FINGERPRINT_SIZE: usize = 32;

/// Size of the `HandshakeInit` body before the trailing signature.
///   32 (fp_A) + 32 (eph_pub) + 32 (pw_salt) = 96 bytes
const HANDSHAKE_INIT_BODY_SIZE: usize = FINGERPRINT_SIZE + 32 + 32;

/// Size of the `HandshakeResp` body before the trailing signature.
///   32 (fp_B) + 32 (eph_pub) = 64 bytes
const HANDSHAKE_RESP_BODY_SIZE: usize = FINGERPRINT_SIZE + 32;

/// Network packet for device discovery.
///
/// Wire format (little-endian, total 628 bytes):
/// ```text
/// [ 0.. 64]  device name  (64 bytes, zero-padded)
/// [64.. 66]  device name length (u16 LE)
/// [66.. 68]  transfer port (u16 LE)
/// [68.. 70]  RSA public key DER length (u16 LE)
/// [70..620]  RSA public key DER (zero-padded to RSA_PUB_DER_MAX = 550 bytes)
/// [620..628] CRC64-NVME checksum (8 bytes)
/// ```
#[derive(Debug, Clone)]
pub struct DiscoveryPacket {
    pub device_name: FixedStr<64>,
    pub transfer_port: u16,
    rsa_pub_key_der: [u8; RSA_PUB_DER_MAX],
    rsa_pub_key_len: u16,
    pub checksum: [u8; 8],
}

impl DiscoveryPacket {
    pub fn new(
        device_name: &str,
        transfer_port: u16,
        rsa_public_key: &RsaPublicKey,
    ) -> ah::Result<Self> {
        let der = public_key_to_der(rsa_public_key)?;
        if der.len() > RSA_PUB_DER_MAX {
            return Err(err!("RSA public key DER too large: {}", der.len()));
        }
        let mut rsa_pub_key_der = [0u8; RSA_PUB_DER_MAX];
        rsa_pub_key_der[..der.len()].copy_from_slice(&der);
        let rsa_pub_key_len = der.len() as u16;
        let device_name_fixed = FixedStr::from_str_trunc(device_name);
        let checksum = Self::compute_checksum(device_name_fixed.as_bytes(), transfer_port, &der);
        Ok(Self {
            device_name: device_name_fixed,
            transfer_port,
            rsa_pub_key_der,
            rsa_pub_key_len,
            checksum,
        })
    }

    pub const fn size() -> usize {
        628
    }

    pub fn serialize(&self) -> ah::Result<Vec<u8>> {
        let mut buf = vec![0u8; Self::size()];
        let name_bytes = self.device_name.as_bytes();
        buf[0..name_bytes.len()].copy_from_slice(name_bytes);
        let name_len = name_bytes.len() as u16;
        buf[64..66].copy_from_slice(&name_len.to_le_bytes());
        buf[66..68].copy_from_slice(&self.transfer_port.to_le_bytes());
        buf[68..70].copy_from_slice(&self.rsa_pub_key_len.to_le_bytes());
        buf[70..620].copy_from_slice(&self.rsa_pub_key_der);
        buf[620..628].copy_from_slice(&self.checksum);
        Ok(buf)
    }

    pub fn deserialize(buf: &[u8]) -> ah::Result<Self> {
        if buf.len() < Self::size() {
            return Err(err!(
                "DiscoveryPacket: buffer too short ({} < {})",
                buf.len(),
                Self::size()
            ));
        }
        let name_len =
            u16::from_le_bytes(buf[64..66].try_into().expect("slice length mismatch")) as usize;
        if name_len > 64 {
            return Err(err!(
                "DiscoveryPacket: device_name_len too large: {name_len}"
            ));
        }
        let name_str = std::str::from_utf8(&buf[0..name_len])
            .context("DiscoveryPacket: device name is not valid UTF-8")?;
        let device_name = FixedStr::from_str_trunc(name_str);
        let transfer_port =
            u16::from_le_bytes(buf[66..68].try_into().expect("slice length mismatch"));
        let rsa_pub_key_len =
            u16::from_le_bytes(buf[68..70].try_into().expect("slice length mismatch")) as usize;
        if rsa_pub_key_len > RSA_PUB_DER_MAX {
            return Err(err!(
                "DiscoveryPacket: RSA key DER length too large: {rsa_pub_key_len}"
            ));
        }
        let mut rsa_pub_key_der = [0u8; RSA_PUB_DER_MAX];
        rsa_pub_key_der.copy_from_slice(&buf[70..620]);
        let mut checksum = [0u8; 8];
        checksum.copy_from_slice(&buf[620..628]);
        Ok(Self {
            device_name,
            transfer_port,
            rsa_pub_key_der,
            rsa_pub_key_len: rsa_pub_key_len as u16,
            checksum,
        })
    }

    /// Extract the RSA public key from the discovery packet.
    pub fn rsa_public_key(&self) -> ah::Result<RsaPublicKey> {
        let len = self.rsa_pub_key_len as usize;
        public_key_from_der(&self.rsa_pub_key_der[..len])
            .context("DiscoveryPacket: Failed to decode RSA public key")
    }

    fn compute_checksum(device_name: &[u8], transfer_port: u16, rsa_pub_key_der: &[u8]) -> [u8; 8] {
        let mut cs = checksum_new();
        cs.update(device_name);
        cs.update(&transfer_port.to_le_bytes());
        cs.update(rsa_pub_key_der);
        cs.finalize().to_le_bytes()
    }

    pub fn verify_checksum(&self) -> bool {
        let len = self.rsa_pub_key_len as usize;
        self.checksum
            == Self::compute_checksum(
                self.device_name.as_bytes(),
                self.transfer_port,
                &self.rsa_pub_key_der[..len],
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

// ---------------------------------------------------------------------------
// Handshake packets for key exchange (sent before encrypted communication)
//
// Neither packet is encrypted: x25519 public keys and Argon2 salts are not
// sensitive and require no confidentiality protection.
//
// Both packets carry an RSA-PSS-SHA3-256 signature over their full body so
// the receiver can authenticate the sender before acting on the contents:
//   HandshakeInit  is signed with the INITIATOR's static RSA private key.
//   HandshakeResp  is signed with the RESPONDER's static RSA private key.
//
// The full public-key DER is not transmitted in the handshake packets.
// Instead, only the 32-byte SHA3-256 fingerprint is included:
//   - The receiver must already hold the peer's public key either in its
//     persistent keystore or in the in-memory discovery map.
//   - The keystore is checked first and preferred over the in-memory copy.
//   - If the key cannot be found in either store, the handshake is aborted.
// ---------------------------------------------------------------------------

/// Sent by the *initiator* (connector) to start the key-exchange handshake.
///
/// Wire format (total 608 bytes):
/// ```text
/// [ 0.. 32]  sender RSA public key fingerprint (SHA3-256, 32 bytes)
/// [32.. 64]  sender ephemeral x25519 public key (32 bytes, cleartext)
/// [64.. 96]  Argon2 password salt (32 bytes, cleartext)
/// [96..608]  RSA-PSS-SHA3-256 signature (512 bytes), over bytes [0..96], signed by sender
/// ```
/// The receiver looks up the sender's public key by fingerprint (keystore first,
/// then in-memory discovery map) and uses it to verify the signature.
pub const HANDSHAKE_INIT_SIZE: usize = HANDSHAKE_INIT_BODY_SIZE + RSA_SIGNATURE_SIZE;

pub struct HandshakeInit {
    /// SHA3-256 fingerprint of the sender's static RSA public key.
    pub sender_fingerprint: [u8; FINGERPRINT_SIZE],
    /// Sender's ephemeral x25519 public key.
    pub ephemeral_public: [u8; 32],
    /// Salt for Argon2 password stretching (random; generated each session).
    pub password_salt: [u8; 32],
}

impl HandshakeInit {
    /// Extract the sender fingerprint from a raw wire buffer without verifying
    /// anything else. Call this to perform the key lookup before `deserialize`.
    pub fn sender_fingerprint_from_buf(buf: &[u8]) -> ah::Result<[u8; FINGERPRINT_SIZE]> {
        if buf.len() < HANDSHAKE_INIT_SIZE {
            return Err(err!(
                "HandshakeInit: buffer too short for fingerprint read ({} < {HANDSHAKE_INIT_SIZE})",
                buf.len()
            ));
        }
        Ok(buf[0..FINGERPRINT_SIZE]
            .try_into()
            .expect("slice length mismatch"))
    }

    /// Serialize the packet and sign it with `our_private_key`.
    pub fn serialize(&self, our_private_key: &RsaPrivateKey) -> ah::Result<Vec<u8>> {
        let mut out = vec![0u8; HANDSHAKE_INIT_SIZE];
        // [0..32]  sender fingerprint
        out[0..FINGERPRINT_SIZE].copy_from_slice(&self.sender_fingerprint);
        // [32..64] ephemeral x25519 public key
        out[FINGERPRINT_SIZE..FINGERPRINT_SIZE + 32].copy_from_slice(&self.ephemeral_public);
        // [64..96] password salt
        out[FINGERPRINT_SIZE + 32..HANDSHAKE_INIT_BODY_SIZE].copy_from_slice(&self.password_salt);
        // [96..608] RSA-PSS signature over [0..96]
        let sig = rsa_sign(our_private_key, &out[..HANDSHAKE_INIT_BODY_SIZE])?;
        if sig.len() != RSA_SIGNATURE_SIZE {
            return Err(err!(
                "HandshakeInit: unexpected RSA signature length: {}",
                sig.len()
            ));
        }
        out[HANDSHAKE_INIT_BODY_SIZE..].copy_from_slice(&sig);
        Ok(out)
    }

    /// Verify the RSA-PSS signature using the already-looked-up `sender_public_key`
    /// and deserialize the packet fields.
    ///
    /// The caller is responsible for first reading the sender fingerprint via
    /// `sender_fingerprint_from_buf`, looking up the corresponding public key
    /// (keystore first, then in-memory discovery map), and passing it here.
    pub fn deserialize(buf: &[u8], sender_public_key: &RsaPublicKey) -> ah::Result<Self> {
        if buf.len() < HANDSHAKE_INIT_SIZE {
            return Err(err!(
                "HandshakeInit: buffer too short ({} < {HANDSHAKE_INIT_SIZE})",
                buf.len()
            ));
        }
        // Verify signature over [0..HANDSHAKE_INIT_BODY_SIZE] before reading fields.
        rsa_verify(
            sender_public_key,
            &buf[..HANDSHAKE_INIT_BODY_SIZE],
            &buf[HANDSHAKE_INIT_BODY_SIZE..HANDSHAKE_INIT_SIZE],
        )
        .context("HandshakeInit: signature verification failed")?;
        // Read cleartext fields.
        let sender_fingerprint: [u8; FINGERPRINT_SIZE] = buf[0..FINGERPRINT_SIZE]
            .try_into()
            .expect("slice length mismatch");
        let ephemeral_public: [u8; 32] = buf[FINGERPRINT_SIZE..FINGERPRINT_SIZE + 32]
            .try_into()
            .expect("slice length mismatch");
        let password_salt: [u8; 32] = buf[FINGERPRINT_SIZE + 32..HANDSHAKE_INIT_BODY_SIZE]
            .try_into()
            .expect("slice length mismatch");
        Ok(Self {
            sender_fingerprint,
            ephemeral_public,
            password_salt,
        })
    }
}

/// Sent by the *responder* (listener) in reply to `HandshakeInit`.
///
/// Wire format (total 576 bytes):
/// ```text
/// [ 0.. 32]  responder RSA public key fingerprint (SHA3-256, 32 bytes)
/// [32.. 64]  responder ephemeral x25519 public key (32 bytes, cleartext)
/// [64..576]  RSA-PSS-SHA3-256 signature (512 bytes), over bytes [0..64], signed by responder
/// ```
/// The receiver looks up the responder's public key by fingerprint (keystore first,
/// then in-memory discovery map) and uses it to verify the signature.
pub const HANDSHAKE_RESP_SIZE: usize = HANDSHAKE_RESP_BODY_SIZE + RSA_SIGNATURE_SIZE;

pub struct HandshakeResp {
    /// SHA3-256 fingerprint of the responder's static RSA public key.
    pub responder_fingerprint: [u8; FINGERPRINT_SIZE],
    /// Responder's ephemeral x25519 public key.
    pub ephemeral_public: [u8; 32],
}

impl HandshakeResp {
    /// Extract the responder fingerprint from a raw wire buffer without verifying
    /// anything else. Call this to perform the key lookup before `deserialize`.
    pub fn responder_fingerprint_from_buf(buf: &[u8]) -> ah::Result<[u8; FINGERPRINT_SIZE]> {
        if buf.len() < HANDSHAKE_RESP_SIZE {
            return Err(err!(
                "HandshakeResp: buffer too short for fingerprint read ({} < {HANDSHAKE_RESP_SIZE})",
                buf.len()
            ));
        }
        Ok(buf[0..FINGERPRINT_SIZE]
            .try_into()
            .expect("slice length mismatch"))
    }

    /// Serialize the packet and sign it with `our_private_key`.
    pub fn serialize(&self, our_private_key: &RsaPrivateKey) -> ah::Result<Vec<u8>> {
        let mut out = vec![0u8; HANDSHAKE_RESP_SIZE];
        // [0..32]  responder fingerprint
        out[0..FINGERPRINT_SIZE].copy_from_slice(&self.responder_fingerprint);
        // [32..64] ephemeral x25519 public key
        out[FINGERPRINT_SIZE..HANDSHAKE_RESP_BODY_SIZE].copy_from_slice(&self.ephemeral_public);
        // [64..576] RSA-PSS signature over [0..64]
        let sig = rsa_sign(our_private_key, &out[..HANDSHAKE_RESP_BODY_SIZE])?;
        if sig.len() != RSA_SIGNATURE_SIZE {
            return Err(err!(
                "HandshakeResp: unexpected RSA signature length: {}",
                sig.len()
            ));
        }
        out[HANDSHAKE_RESP_BODY_SIZE..].copy_from_slice(&sig);
        Ok(out)
    }

    /// Verify the RSA-PSS signature using the already-looked-up `responder_public_key`
    /// and deserialize the packet fields.
    ///
    /// The caller is responsible for first reading the responder fingerprint via
    /// `responder_fingerprint_from_buf`, looking up the corresponding public key
    /// (keystore first, then in-memory discovery map), and passing it here.
    pub fn deserialize(buf: &[u8], responder_public_key: &RsaPublicKey) -> ah::Result<Self> {
        if buf.len() < HANDSHAKE_RESP_SIZE {
            return Err(err!(
                "HandshakeResp: buffer too short ({} < {HANDSHAKE_RESP_SIZE})",
                buf.len()
            ));
        }
        // Verify signature over [0..HANDSHAKE_RESP_BODY_SIZE] before reading fields.
        rsa_verify(
            responder_public_key,
            &buf[..HANDSHAKE_RESP_BODY_SIZE],
            &buf[HANDSHAKE_RESP_BODY_SIZE..HANDSHAKE_RESP_SIZE],
        )
        .context("HandshakeResp: signature verification failed")?;
        // Read cleartext fields.
        let responder_fingerprint: [u8; FINGERPRINT_SIZE] = buf[0..FINGERPRINT_SIZE]
            .try_into()
            .expect("slice length mismatch");
        let ephemeral_public: [u8; 32] = buf[FINGERPRINT_SIZE..HANDSHAKE_RESP_BODY_SIZE]
            .try_into()
            .expect("slice length mismatch");
        Ok(Self {
            responder_fingerprint,
            ephemeral_public,
        })
    }
}

// ---------------------------------------------------------------------------
// Encrypted packet: authenticated header + encrypted payload
// ---------------------------------------------------------------------------

/// Acceptable clock skew for replay-attack prevention: 5 minutes.
pub const TIMESTAMP_WINDOW_SECS: u64 = 300;

/// Header of an encrypted transfer packet (authenticated but not encrypted).
///
/// Wire format (all little-endian):
/// ```text
/// [ 0.. 8]  timestamp (microseconds since Unix epoch, u64 LE)
/// [ 8..20]  AES-GCM nonce (12 bytes)
/// [20..24]  ciphertext length (u32 LE, includes 16-byte AES-GCM auth tag)
/// ```
/// Total: 24 bytes.
pub const ENC_HEADER_SIZE: usize = 24;

pub struct EncryptedPacketHeader {
    /// Microseconds since Unix epoch.
    pub timestamp_us: u64,
    pub nonce: [u8; NONCE_LEN],
    /// Ciphertext length in bytes, including the 16-byte AES-GCM authentication tag.
    pub ciphertext_len: u32,
}

impl EncryptedPacketHeader {
    pub fn new(nonce: [u8; NONCE_LEN], ciphertext_len: u32) -> Self {
        let timestamp_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        Self {
            timestamp_us,
            nonce,
            ciphertext_len,
        }
    }

    pub fn to_bytes(&self) -> [u8; ENC_HEADER_SIZE] {
        let mut buf = [0u8; ENC_HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.timestamp_us.to_le_bytes());
        buf[8..20].copy_from_slice(&self.nonce);
        buf[20..24].copy_from_slice(&self.ciphertext_len.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> ah::Result<Self> {
        if buf.len() < ENC_HEADER_SIZE {
            return Err(err!(
                "EncryptedPacketHeader: buffer too short ({} < {ENC_HEADER_SIZE})",
                buf.len()
            ));
        }
        let timestamp_us = u64::from_le_bytes(buf[0..8].try_into().expect("slice length mismatch"));
        let nonce: [u8; NONCE_LEN] = buf[8..20].try_into().expect("slice length mismatch");
        let ciphertext_len =
            u32::from_le_bytes(buf[20..24].try_into().expect("slice length mismatch"));
        Ok(Self {
            timestamp_us,
            nonce,
            ciphertext_len,
        })
    }

    /// Verify that the timestamp is within `TIMESTAMP_WINDOW_SECS` of now.
    pub fn verify_timestamp(&self) -> bool {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        let window_us = TIMESTAMP_WINDOW_SECS * 1_000_000;
        self.timestamp_us.abs_diff(now_us) <= window_us
    }
}
