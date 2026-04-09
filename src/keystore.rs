//! Peer identity key store.
//!
//! Persists peer RSA public-key fingerprints keyed by key fingerprint (SHA3-256 hex).
//! On first approved connection with a new peer the fingerprint is stored; on subsequent
//! connections the stored fingerprint is compared against the presented one.
//!
//! Also manages the local device's own static RSA key pair, generating it once
//! and persisting it encrypted to disk.

use crate::crypto::{
    RSA_BITS, RsaPrivateKey, RsaPublicKey, fingerprint_hex, generate_rsa_keypair,
    private_key_from_der, private_key_to_der, public_key_fingerprint, public_key_from_der,
    public_key_to_der, rsa_key_bits,
};
use anyhow::{self as ah, Context as _, format_err as err};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const IDENTITY_FILE: &str = "identity.der";
const KNOWN_PEERS_FILE: &str = "known_peers.json";

/// A stored entry about a known remote peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownPeer {
    /// Human-readable device name at time of first contact (informational only).
    pub device_name: String,
    /// Hex-encoded SHA3-256 fingerprint of the peer's RSA public key.
    pub fingerprint: String,
    /// Base64-encoded DER of the peer's RSA public key.
    pub public_key_b64: String,
}

/// Outcome of a peer key check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCheckResult {
    /// Peer is unknown; not yet stored in the keystore.
    FirstContact,
    /// Peer is known and the fingerprint matches.
    Trusted,
    /// Peer is known but the fingerprint does NOT match - potential MITM.
    FingerprintMismatch { stored: String, presented: String },
}

/// On-disk format for peer storage.
#[derive(Debug, Default, Serialize, Deserialize)]
struct KnownPeersFile {
    peers: HashMap<String, KnownPeer>,
}

/// Returns the application data directory for persisting keys.
fn data_dir() -> ah::Result<PathBuf> {
    #[cfg(target_os = "android")]
    {
        // The `dirs` crate does not support Android; use `Context.getFilesDir()` instead.
        // That already returns an app-scoped path like /data/user/0/<pkg>/files.
        return crate::android_interface::android_get_files_dir()
            .ok_or_else(|| err!("Could not determine Android files directory"));
    }
    #[cfg(not(target_os = "android"))]
    {
        let base = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| err!("Could not determine user data directory"))?;
        Ok(base.join("ch.bues.transfer"))
    }
}

/// Load the local device's static RSA private key, generating and persisting it if needed.
pub fn load_or_generate_identity() -> ah::Result<(RsaPrivateKey, RsaPublicKey)> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).context("Failed to create keys directory")?;
    let identity_path = dir.join(IDENTITY_FILE);

    if identity_path.exists() {
        let der = std::fs::read(&identity_path).context("Failed to read identity file")?;
        let private_key = private_key_from_der(&der)?;
        if rsa_key_bits(&private_key) != RSA_BITS {
            log::warn!(
                "Identity key at {} has wrong size ({} bits, expected {RSA_BITS}); \
                 regenerating.",
                identity_path.display(),
                rsa_key_bits(&private_key),
            );
            std::fs::remove_file(&identity_path)
                .context("Failed to remove outdated identity file")?;
        } else {
            let public_key = RsaPublicKey::from(&private_key);
            log::debug!("Loaded existing identity from {}", identity_path.display());
            return Ok((private_key, public_key));
        }
    }

    log::info!("Generating new RSA-{RSA_BITS} identity key pair...");
    let (private_key, public_key) = generate_rsa_keypair()?;
    let der = private_key_to_der(&private_key)?;
    write_private_file(&identity_path, &der).context("Failed to write identity file")?;
    log::info!("Saved new identity to {}", identity_path.display());
    Ok((private_key, public_key))
}

/// Write a file with restrictive permissions (owner-read-only on Unix).
fn write_private_file(path: &Path, data: &[u8]) -> ah::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?
            .write_all(data)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
    }
    Ok(())
}

#[cfg(unix)]
use std::io::Write as _;

/// Load the known-peers file from disk.
fn load_known_peers(path: &Path) -> ah::Result<KnownPeersFile> {
    if !path.exists() {
        return Ok(KnownPeersFile::default());
    }
    let json = std::fs::read_to_string(path).context("Failed to read known_peers file")?;
    serde_json::from_str(&json).context("Failed to parse known_peers file")
}

/// Persist the known-peers file to disk.
fn save_known_peers(path: &Path, kpf: &KnownPeersFile) -> ah::Result<()> {
    let json = serde_json::to_string_pretty(kpf).context("Failed to serialize known_peers")?;
    std::fs::write(path, json).context("Failed to write known_peers file")
}

fn known_peers_path() -> ah::Result<PathBuf> {
    Ok(data_dir()?.join(KNOWN_PEERS_FILE))
}

/// Check the stored identity for a remote peer.
///
/// Only reads the keystore - never writes.
/// Returns `KeyCheckResult::FirstContact` if unknown, `Trusted` if the fingerprint
/// matches, or `FingerprintMismatch` if it does not.
/// The caller is responsible for persisting the key after a successful handshake.
pub fn check_peer_key(presented_public_key: &RsaPublicKey) -> ah::Result<KeyCheckResult> {
    let path = known_peers_path()?;

    let kpf = load_known_peers(&path)?;

    let presented_fp: [u8; 32] = public_key_fingerprint(presented_public_key)?;
    let presented_fp_hex = fingerprint_hex(&presented_fp);

    if let Some(known) = kpf.peers.get(&presented_fp_hex) {
        if known.fingerprint == presented_fp_hex {
            return Ok(KeyCheckResult::Trusted);
        } else {
            return Ok(KeyCheckResult::FingerprintMismatch {
                stored: known.fingerprint.clone(),
                presented: presented_fp_hex,
            });
        }
    }

    Ok(KeyCheckResult::FirstContact)
}

/// Look up the stored public key for a peer, if any.
pub fn get_known_peer_public_key(fingerprint: &[u8; 32]) -> ah::Result<Option<RsaPublicKey>> {
    let path = known_peers_path()?;
    let kpf = load_known_peers(&path)?;
    if let Some(known) = kpf.peers.get(&fingerprint_hex(fingerprint)) {
        let der = B64
            .decode(&known.public_key_b64)
            .context("Base64 decode failed")?;
        let key = public_key_from_der(&der)?;
        Ok(Some(key))
    } else {
        Ok(None)
    }
}

/// Override the stored key for a peer (used when the user explicitly accepts a changed key).
pub fn trust_peer(device_name: &str, presented_public_key: &RsaPublicKey) -> ah::Result<()> {
    let path = known_peers_path()?;
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).context("Failed to create keys directory")?;

    let mut kpf = load_known_peers(&path)?;
    let presented_fp: [u8; 32] = public_key_fingerprint(presented_public_key)?;
    let pub_der = public_key_to_der(presented_public_key)?;

    kpf.peers.insert(
        fingerprint_hex(&presented_fp),
        KnownPeer {
            device_name: device_name.to_string(),
            fingerprint: fingerprint_hex(&presented_fp),
            public_key_b64: B64.encode(&pub_der),
        },
    );
    save_known_peers(&path, &kpf)
}
