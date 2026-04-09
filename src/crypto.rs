//! Cryptographic primitives for the transfer protocol.
//!
//! Key derivation flow (per session):
//!  1. Each peer generates an ephemeral x25519 key pair.
//!  2. A DH shared secret is derived from the ephemeral keys.
//!  3. Optionally, the user password is stretched to 32 bytes via Argon2id (slow/high-mem).
//!  4. The ephemeral public key is used as binding material; it is encrypted with the remote
//!     peer's static RSA public key and also with our own, so both identities are bound.
//!  5. All inputs are combined via a fast Argon2id pass to produce the 32-byte session key.

use aes_gcm::{
    Aes256Gcm, KeyInit as _, Nonce as GcmNonce,
    aead::{Aead as _, Payload as AeadPayload},
};
use anyhow::{self as ah, Context as _, format_err as err};
use argon2::{Algorithm, Argon2, Params, Version};
use rsa::{
    pkcs8::{
        DecodePrivateKey as _, DecodePublicKey as _, EncodePrivateKey as _, EncodePublicKey as _,
    },
    pss::{BlindedSigningKey, Signature as PssSignature, VerifyingKey as PssVerifyingKey},
    signature::{RandomizedSigner as _, SignatureEncoding as _, Verifier as _},
    traits::PublicKeyParts as _,
};
use sha3::{Digest, Sha3_256};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};
use zeroize::Zeroize as _;

pub use rsa::{RsaPrivateKey, RsaPublicKey};

/// Size of the AES-256-GCM key in bytes.
pub const SESSION_KEY_LEN: usize = 32;
/// AES-GCM nonce size in bytes.
pub const NONCE_LEN: usize = 12;
/// RSA key bits.
pub const RSA_BITS: usize = 4096;

// Argon2id parameters for password stretching: ~500 ms, ~64 MiB.
const PW_KDF_MEM_KIB: u32 = 65_536;
const PW_KDF_TIME: u32 = 3;
const PW_KDF_THREADS: u32 = 4;

// Argon2id parameters for session key mixing: ~10 ms, ~128 KiB.
const MIX_KDF_MEM_KIB: u32 = 128;
const MIX_KDF_TIME: u32 = 1;
const MIX_KDF_THREADS: u32 = 1;

/// A 32-byte session key derived for AES-GCM encryption.
#[derive(Clone)]
pub struct SessionKey(pub [u8; SESSION_KEY_LEN]);

impl Drop for SessionKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey([redacted])")
    }
}

/// Generate a fresh RSA-4096 static key pair.
pub fn generate_rsa_keypair() -> ah::Result<(RsaPrivateKey, RsaPublicKey)> {
    let mut rng = rand_core::OsRng;
    let private_key =
        RsaPrivateKey::new(&mut rng, RSA_BITS).context("RSA key generation failed")?;
    let public_key = RsaPublicKey::from(&private_key);
    Ok((private_key, public_key))
}

/// Returns the modulus size of the private key in bits.
pub fn rsa_key_bits(key: &RsaPrivateKey) -> usize {
    key.size() * 8
}

/// Serialize an RSA private key to PKCS#8 DER bytes.
pub fn private_key_to_der(key: &RsaPrivateKey) -> ah::Result<Vec<u8>> {
    Ok(key
        .to_pkcs8_der()
        .context("Failed to serialize RSA private key")?
        .as_bytes()
        .to_vec())
}

/// Deserialize an RSA private key from PKCS#8 DER bytes.
pub fn private_key_from_der(der: &[u8]) -> ah::Result<RsaPrivateKey> {
    RsaPrivateKey::from_pkcs8_der(der).context("Failed to deserialize RSA private key")
}

/// Serialize an RSA public key to PKCS#8 DER (SubjectPublicKeyInfo) bytes.
pub fn public_key_to_der(key: &RsaPublicKey) -> ah::Result<Vec<u8>> {
    Ok(key
        .to_public_key_der()
        .context("Failed to serialize RSA public key")?
        .into_vec())
}

/// Deserialize an RSA public key from DER bytes.
pub fn public_key_from_der(der: &[u8]) -> ah::Result<RsaPublicKey> {
    RsaPublicKey::from_public_key_der(der).context("Failed to deserialize RSA public key")
}

/// SHA3-256 fingerprint of an RSA public key (for display and peer identity verification).
pub fn public_key_fingerprint(key: &RsaPublicKey) -> ah::Result<[u8; 32]> {
    let der = public_key_to_der(key)?;
    let hash: [u8; 32] = Sha3_256::digest(&der).into();
    Ok(hash)
}

/// Hex-encode a fingerprint with colon-separated bytes for human-readable display.
pub fn fingerprint_hex(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Stretch a user password to 32 bytes using Argon2id (slow, high-memory).
/// Returns `None` if `password` is empty.
pub fn stretch_password(password: &str, salt: &[u8; 32]) -> ah::Result<Option<[u8; 32]>> {
    if password.is_empty() {
        return Ok(None);
    }
    let params = Params::new(PW_KDF_MEM_KIB, PW_KDF_TIME, PW_KDF_THREADS, Some(32))
        .map_err(|e| err!("Argon2 params (password): {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut output)
        .map_err(|e| err!("Argon2 password hashing failed: {e}"))?;
    Ok(Some(output))
}

/// Sign `message` using RSA-PSS with SHA3-256.
/// Returns a fixed-length signature (modulus size in bytes).
pub fn rsa_sign(private_key: &RsaPrivateKey, message: &[u8]) -> ah::Result<Vec<u8>> {
    let signing_key = BlindedSigningKey::<Sha3_256>::new(private_key.clone());
    let sig = signing_key.sign_with_rng(&mut rand_core::OsRng, message);
    Ok(sig.to_bytes().into_vec())
}

/// Verify an RSA-PSS-SHA3-256 `signature` over `message`.
pub fn rsa_verify(public_key: &RsaPublicKey, message: &[u8], signature: &[u8]) -> ah::Result<()> {
    let verifying_key = PssVerifyingKey::<Sha3_256>::new(public_key.clone());
    let sig = PssSignature::try_from(signature)
        .map_err(|_| err!("RSA-PSS signature has incorrect length"))?;
    verifying_key
        .verify(message, &sig)
        .map_err(|e| err!("RSA-PSS signature verification failed: {e}"))
}

/// Argon2id salt constant for session key derivation.
/// A fixed domain-separation string; all entropy comes from the password (kdf_input).
const SESSION_KDF_SALT: &[u8] = b"ch.bues.transfer.session.key.v1";

/// Derive the 32-byte AES-GCM session key from all available secrets.
///
/// Both peers call this with the same arguments (in the same order) so they
/// independently produce an identical session key.
///
/// # Arguments
/// * `dh_shared_secret` - 32-byte x25519 shared secret (same on both sides).
/// * `initiator_rsa_pub` - Initiator's static RSA public key (sent in HandshakeInit).
/// * `responder_rsa_pub` - Responder's static RSA public key (sent in HandshakeResp).
/// * `stretched_password` - Optional pre-stretched (Argon2id) password secret.
///
/// The RSA public keys are included as their DER bytes (deterministic) so both
/// peers produce identical input bytes without exchanging additional data.
/// This binds the ephemeral DH secret, both long-term RSA identities, and the
/// optional password into the session key.
pub fn derive_session_key(
    dh_shared_secret: &[u8; 32],
    initiator_rsa_pub: &RsaPublicKey,
    responder_rsa_pub: &RsaPublicKey,
    stretched_password: Option<&[u8; 32]>,
) -> ah::Result<SessionKey> {
    let initiator_der = public_key_to_der(initiator_rsa_pub)?;
    let responder_der = public_key_to_der(responder_rsa_pub)?;

    // Assemble KDF input: DH secret || initiator_rsa_der || responder_rsa_der || pw_material
    // pw_material is always included: all-zeros if no password was provided.
    // The order is fixed (initiator first) so both sides produce identical bytes.
    let pw_material: &[u8; 32] = stretched_password.unwrap_or(&[0u8; 32]);
    let mut kdf_input: Vec<u8> = Vec::with_capacity(
        dh_shared_secret.len() + initiator_der.len() + responder_der.len() + pw_material.len(),
    );
    kdf_input.extend_from_slice(dh_shared_secret);
    kdf_input.extend_from_slice(&initiator_der);
    kdf_input.extend_from_slice(&responder_der);
    kdf_input.extend_from_slice(pw_material);

    // Final Argon2id pass: fast, low-memory.
    // kdf_input (containing all secrets) is the password; a constant domain-separation
    // string is the salt. All entropy is in the password, not the salt.
    let params = Params::new(
        MIX_KDF_MEM_KIB,
        MIX_KDF_TIME,
        MIX_KDF_THREADS,
        Some(SESSION_KEY_LEN),
    )
    .map_err(|e| err!("Argon2 params (mix): {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut session_key_bytes = [0u8; SESSION_KEY_LEN];
    argon2
        .hash_password_into(&kdf_input, SESSION_KDF_SALT, &mut session_key_bytes)
        .map_err(|e| err!("Argon2 session key derivation failed: {e}"))?;

    Ok(SessionKey(session_key_bytes))
}

/// Ephemeral x25519 key pair used for a single session.
pub struct EphemeralKeyPair {
    secret: EphemeralSecret,
    pub public: X25519PublicKey,
}

impl EphemeralKeyPair {
    pub fn generate() -> Self {
        let secret = EphemeralSecret::random_from_rng(rand_core::OsRng);
        let public = X25519PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Return the ephemeral public key bytes used as KDF binding input.
    pub fn public_bytes(&self) -> [u8; 32] {
        self.public.to_bytes()
    }

    /// Consume this key pair and perform DH to obtain the 32-byte shared secret.
    pub fn diffie_hellman(self, remote_public: &X25519PublicKey) -> [u8; 32] {
        self.secret.diffie_hellman(remote_public).to_bytes()
    }
}

/// Encrypt `plaintext` with AES-256-GCM.
/// `aad` is authenticated-but-not-encrypted additional data (the packet header bytes).
/// Returns ciphertext with the 16-byte authentication tag appended.
pub fn aes_gcm_encrypt(
    key: &SessionKey,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> ah::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(&key.0).context("AES-GCM key init")?;
    let nonce = GcmNonce::from_slice(nonce);
    cipher
        .encrypt(
            nonce,
            AeadPayload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| err!("AES-GCM encryption failed: {e}"))
}

/// Decrypt and authenticate `ciphertext` (ciphertext || 16-byte tag) with AES-256-GCM.
pub fn aes_gcm_decrypt(
    key: &SessionKey,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> ah::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(&key.0).context("AES-GCM key init")?;
    let nonce = GcmNonce::from_slice(nonce);
    cipher
        .decrypt(
            nonce,
            AeadPayload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|e| err!("AES-GCM decryption/authentication failed: {e}"))
}

/// Generate a cryptographically random 12-byte AES-GCM nonce.
pub fn random_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("getrandom failed");
    nonce
}

/// Generate a cryptographically random 32-byte salt.
pub fn random_salt() -> [u8; 32] {
    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt).expect("getrandom failed");
    salt
}
