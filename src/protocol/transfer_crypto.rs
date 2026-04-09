//! Crypto handshake and encrypted-message framing for the transfer protocol.

use crate::{
    crypto::{
        EphemeralKeyPair, RsaPublicKey, SessionKey, aes_gcm_decrypt, aes_gcm_encrypt,
        derive_session_key, fingerprint_hex, public_key_fingerprint, random_nonce, random_salt,
        stretch_password,
    },
    ipc::{SessionPassword, TransferEvent},
    keystore::{
        KeyCheckResult, check_peer_key, get_known_peer_public_key, load_or_generate_identity,
        trust_peer,
    },
    protocol::{
        discovery::DeviceMap,
        packets::{
            ENC_HEADER_SIZE, EncryptedPacketHeader, HANDSHAKE_INIT_SIZE, HANDSHAKE_RESP_SIZE,
            HandshakeInit, HandshakeResp,
        },
    },
};
use anyhow::{self as ah, Context as _, format_err as err};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
    sync::{Mutex, mpsc, oneshot},
    time::timeout,
};
use x25519_dalek::PublicKey as X25519PublicKey;

/// Timeout for performing the crypto handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Timeout for waiting for the handshake response after sending the handshake init.
const HANDSHAKE_RESP_TIMEOUT: Duration = Duration::from_secs(120);
/// Timeout for waiting for a peer decision from the user.
const PEER_DECISION_TIMEOUT: Duration = Duration::from_secs(120);

/// A transfer waiting on the user's accept/reject decision.
pub(super) struct PendingPeerDecision {
    #[allow(dead_code)]
    pub(super) transfer_id: u64,
    /// Channel to send the user's decision back to the connection handler.
    pub(super) decision_tx: oneshot::Sender<bool>,
    /// The peer's key fingerprint (hex-encoded SHA3-256 of their RSA public key DER).
    pub(super) peer_name: String,
    pub(super) peer_key: RsaPublicKey,
}

// ---------------------------------------------------------------------------
// Peer key lookup helper
// ---------------------------------------------------------------------------

/// Look up a peer's RSA public key by fingerprint.
/// The keystore is checked first (preferred, persistent); if not stored yet,
/// fall back to the in-memory discovery map. Returns an error if the key
/// cannot be found in either store.
async fn resolve_peer_key(fp: &[u8; 32], device_map: &DeviceMap) -> ah::Result<RsaPublicKey> {
    let fp_copy = *fp;
    if let Some(key) = tokio::task::spawn_blocking(move || get_known_peer_public_key(&fp_copy))
        .await
        .context("Keystore lookup task panicked")?
        .context("Keystore lookup failed")?
    {
        return Ok(key);
    }
    if let Some(device) = device_map.lock().await.get(fp) {
        return Ok(device.rsa_public_key.clone());
    }
    Err(err!(
        "Peer with fingerprint {} is unknown - \
         not in keystore and not in in-memory discovery map",
        fingerprint_hex(fp)
    ))
}

// ---------------------------------------------------------------------------
// Encrypted message framing helpers
//
// Wire format for each message:
//   [ENC_HEADER_SIZE bytes] encrypted packet header (timestamp + nonce + len)
//   [ciphertext_len bytes]  AES-GCM ciphertext with appended authentication tag
//
// The packet header bytes are used as AAD so they are authenticated along with the payload.
// ---------------------------------------------------------------------------

pub(super) async fn send_encrypted_message(
    stream: &mut TcpStream,
    key: &SessionKey,
    plaintext: &[u8],
) -> ah::Result<()> {
    let nonce = random_nonce();
    // AES-GCM tag is always 16 bytes; compute ciphertext_len before encrypting so it
    // can be included in the header, which is used as AAD.
    let ciphertext_len = (plaintext.len() + 16) as u32;
    let header = EncryptedPacketHeader::new(nonce, ciphertext_len);
    let header_bytes = header.to_bytes();
    let ciphertext = aes_gcm_encrypt(key, &nonce, &header_bytes, plaintext)?;
    stream.write_all(&header_bytes).await?;
    stream.write_all(&ciphertext).await?;
    Ok(())
}

/// Receive and authenticate an encrypted message.
///
/// `last_ts_us` must be initialized to `0` at the start of the session and
/// passed by `&mut` on every call.  It enforces that timestamps are
/// monotonically increasing across packets from the same peer, preventing
/// replay attacks even within the 5-minute clock-skew window.
pub(super) async fn recv_encrypted_message(
    stream: &mut TcpStream,
    key: &SessionKey,
    last_ts_us: &mut u64,
) -> ah::Result<Vec<u8>> {
    let mut header_buf = [0u8; ENC_HEADER_SIZE];
    stream
        .read_exact(&mut header_buf)
        .await
        .context("Failed to read encrypted packet header")?;
    let header = EncryptedPacketHeader::from_bytes(&header_buf)?;

    if !header.verify_timestamp() {
        return Err(err!(
            "Encrypted packet timestamp out of acceptable window (replay attack?)"
        ));
    }

    // Monotonically increasing timestamp check.
    if header.timestamp_us <= *last_ts_us {
        return Err(err!(
            "Encrypted packet timestamp is not monotonically increasing (replay attack?)"
        ));
    }
    *last_ts_us = header.timestamp_us;

    let ciphertext_len = header.ciphertext_len as usize;

    // Sanity limit: 256 MiB per chunk.
    if ciphertext_len > 256 * 1024 * 1024 {
        return Err(err!("Ciphertext length too large: {ciphertext_len}"));
    }

    let mut ciphertext = vec![0u8; ciphertext_len];
    stream
        .read_exact(&mut ciphertext)
        .await
        .context("Failed to read ciphertext")?;

    aes_gcm_decrypt(key, &header.nonce, &header_buf, &ciphertext)
}

// ---------------------------------------------------------------------------
// Crypto handshake helpers
// ---------------------------------------------------------------------------

/// Perform the handshake as the *initiator* (sender/connector).
///
/// Sends a `HandshakeInit` containing our fingerprint (not the full key),
/// receives a `HandshakeResp`, verifies its fingerprint matches the expected
/// responder, looks up the responder's public key (keystore-preferred),
/// verifies the signature, then derives and returns the `SessionKey`.
///
/// `responder_rsa_pub_key` must be the key obtained from the responder's
/// discovery packet. It is used as the fallback if the keystore does not yet
/// have an entry for the responder.
pub(super) async fn perform_handshake_as_initiator(
    stream: &mut TcpStream,
    responder_rsa_pub_key: RsaPublicKey,
    session_password: SessionPassword,
    transfer_id: u64,
    event_tx: &mpsc::UnboundedSender<TransferEvent>,
) -> ah::Result<SessionKey> {
    let (our_private_key, our_public_key) = tokio::task::spawn_blocking(load_or_generate_identity)
        .await
        .context("Identity task panicked")?
        .context("Failed to load/generate identity")?;

    let ephemeral = EphemeralKeyPair::generate();
    let ephemeral_pub_bytes = ephemeral.public_bytes();
    let password_salt = random_salt();

    // Build HandshakeInit: send our fingerprint instead of the full public key.
    let our_fp = public_key_fingerprint(&our_public_key)?;
    let init = HandshakeInit {
        sender_fingerprint: our_fp,
        ephemeral_public: ephemeral_pub_bytes,
        password_salt,
    };
    let init_bytes = tokio::task::spawn_blocking(move || init.serialize(&our_private_key))
        .await
        .context("RSA sign task panicked")?
        .context("HandshakeInit serialization failed")?;
    match timeout(HANDSHAKE_TIMEOUT, stream.write_all(&init_bytes)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(err!("Timeout sending handshake init")),
    }

    // Receive HandshakeResp.
    let mut resp_buf = vec![0u8; HANDSHAKE_RESP_SIZE];
    match timeout(HANDSHAKE_RESP_TIMEOUT, stream.read_exact(&mut resp_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(err!("Timeout reading handshake response")),
    }

    // Verify the responder fingerprint in the response matches what we expected.
    let fp_b = HandshakeResp::responder_fingerprint_from_buf(&resp_buf)?;
    let expected_fp = public_key_fingerprint(&responder_rsa_pub_key)?;
    if fp_b != expected_fp {
        return Err(err!(
            "HandshakeResp: responder fingerprint mismatch - potential man-in-the-middle attack"
        ));
    }

    // Key lookup: prefer keystore over the in-memory discovery key.
    let resolved_responder_key = {
        let fp = fp_b;
        match tokio::task::spawn_blocking(move || get_known_peer_public_key(&fp))
            .await
            .context("Keystore lookup task panicked")?
            .context("Keystore lookup failed")?
        {
            Some(k) => k,
            None => responder_rsa_pub_key.clone(),
        }
    };

    let resp = {
        let key = resolved_responder_key.clone();
        tokio::task::spawn_blocking(move || HandshakeResp::deserialize(&resp_buf, &key))
            .await
            .context("RSA verify task panicked")?
            .context("HandshakeResp deserialization failed")?
    };

    // Key check for the responder.
    let peer_fp = resp.responder_fingerprint;
    let peer_name = fingerprint_hex(&peer_fp);
    let check_result = tokio::task::spawn_blocking({
        let peer_key = resolved_responder_key.clone();
        move || check_peer_key(&peer_key)
    })
    .await
    .context("Key check task panicked")?
    .context("Key check failed")?;

    match &check_result {
        KeyCheckResult::FirstContact => {
            let _ = event_tx.send(TransferEvent::NewPeerContact {
                transfer_id,
                fingerprint: peer_name.clone(),
            });
        }
        KeyCheckResult::Trusted => {}
        KeyCheckResult::FingerprintMismatch { stored, presented } => {
            log::warn!(
                "Transfer {transfer_id}: fingerprint mismatch for responder {peer_name}! \
                 stored={stored}, presented={presented}"
            );
            let _ = event_tx.send(TransferEvent::KeyMismatchWarning {
                transfer_id,
                device_name: peer_name.clone(),
                stored_fingerprint: stored.clone(),
                presented_fingerprint: presented.clone(),
                is_incoming: false,
            });
            // Abort the outgoing transfer on mismatch.
            return Err(err!(
                "Fingerprint mismatch for remote peer - potential man-in-the-middle attack"
            ));
        }
    }

    let password = session_password.lock().expect("Lock poisoned").clone();
    let stretched_password = if !password.is_empty() {
        tokio::task::spawn_blocking(move || stretch_password(&password, &password_salt))
            .await
            .context("Password stretch task panicked")?
            .context("Password KDF failed")?
    } else {
        None
    };

    let remote_pub = X25519PublicKey::from(resp.ephemeral_public);
    let dh_secret = ephemeral.diffie_hellman(&remote_pub);

    // Initiator = us (our_public_key), responder = peer (resolved_responder_key).
    let key = {
        let resp_pub = resolved_responder_key.clone();
        tokio::task::spawn_blocking(move || {
            derive_session_key(
                &dh_secret,
                &our_public_key,
                &resp_pub,
                stretched_password.as_ref(),
            )
        })
        .await
        .context("Session key derivation task panicked")?
        .context("Session key derivation failed")?
    };

    // Persist the responder's key to the keystore now that the handshake succeeded.
    if let KeyCheckResult::FirstContact = check_result {
        let peer_key = resolved_responder_key.clone();
        let peer_name_store = peer_name.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || trust_peer(&peer_name_store, &peer_key))
            .await
            .context("Key store task panicked")?
        {
            log::warn!("Failed to persist peer key after first contact: {e}");
        }
    }

    Ok(key)
}

/// Perform the handshake as the *responder* (receiver/listener).
///
/// Receives a `HandshakeInit` containing the initiator's fingerprint,
/// looks up the initiator's public key (keystore first, then in-memory
/// discovery map), verifies the signature, sends a `HandshakeResp`,
/// then derives and returns the `SessionKey`.
///
/// `device_map` is used as the in-memory fallback when the initiator is not
/// yet in the keystore (first connection).
///
/// If the initiator's key fingerprint is mismatched with the stored value,
/// a `KeyMismatchWarning` event is emitted and the function waits for the user's
/// `AcceptKeyChange` / `RejectKeyChange` decision.
pub(super) async fn perform_handshake_as_responder(
    stream: &mut TcpStream,
    transfer_id: u64,
    session_password: SessionPassword,
    event_tx: &mpsc::UnboundedSender<TransferEvent>,
    pending_decisions: &Arc<Mutex<HashMap<u64, PendingPeerDecision>>>,
    device_map: &DeviceMap,
) -> ah::Result<SessionKey> {
    let (our_private_key, our_public_key) = tokio::task::spawn_blocking(load_or_generate_identity)
        .await
        .context("Identity task panicked")?
        .context("Failed to load/generate identity")?;

    // Receive HandshakeInit.
    let mut init_buf = vec![0u8; HANDSHAKE_INIT_SIZE];
    match timeout(HANDSHAKE_TIMEOUT, stream.read_exact(&mut init_buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(err!("Timeout reading handshake init")),
    }

    // Read initiator fingerprint, then look up the key before verifying the signature.
    let fp_a = HandshakeInit::sender_fingerprint_from_buf(&init_buf)?;
    let initiator_rsa_pub = resolve_peer_key(&fp_a, device_map).await?;
    let init = {
        let key = initiator_rsa_pub.clone();
        tokio::task::spawn_blocking(move || HandshakeInit::deserialize(&init_buf, &key))
            .await
            .context("RSA verify task panicked")?
            .context("HandshakeInit deserialization failed")?
    };

    let peer_fp = init.sender_fingerprint;
    let peer_name = fingerprint_hex(&peer_fp);

    // Key check for the initiator.
    let check_result = tokio::task::spawn_blocking({
        let peer_key = initiator_rsa_pub.clone();
        move || check_peer_key(&peer_key)
    })
    .await
    .context("Key check task panicked")?
    .context("Key check failed")?;

    match &check_result {
        KeyCheckResult::FirstContact => {
            // Ask the user to confirm the new peer before proceeding.
            let (decision_tx, decision_rx) = oneshot::channel();
            {
                let mut map = pending_decisions.lock().await;
                map.insert(
                    transfer_id,
                    PendingPeerDecision {
                        transfer_id,
                        decision_tx,
                        peer_name: peer_name.clone(),
                        peer_key: initiator_rsa_pub.clone(),
                    },
                );
            }
            let _ = event_tx.send(TransferEvent::NewPeerContact {
                transfer_id,
                fingerprint: peer_name.clone(),
            });

            let accepted = match timeout(PEER_DECISION_TIMEOUT, decision_rx).await {
                Ok(Ok(v)) => v,
                Ok(Err(_)) => false, // sender dropped
                Err(_) => {
                    pending_decisions.lock().await.remove(&transfer_id);
                    false
                }
            };

            if !accepted {
                return Err(err!("New peer rejected by user - aborting transfer"));
            }
        }
        KeyCheckResult::Trusted => {}
        KeyCheckResult::FingerprintMismatch { stored, presented } => {
            log::warn!(
                "Transfer {transfer_id}: fingerprint mismatch for initiator {peer_name}! \
                 stored={stored}, presented={presented}"
            );
            // Ask the user to decide.
            let (decision_tx, decision_rx) = oneshot::channel();
            {
                let mut map = pending_decisions.lock().await;
                map.insert(
                    transfer_id,
                    PendingPeerDecision {
                        transfer_id,
                        decision_tx,
                        peer_name: peer_name.clone(),
                        peer_key: initiator_rsa_pub.clone(),
                    },
                );
            }
            let _ = event_tx.send(TransferEvent::KeyMismatchWarning {
                transfer_id,
                device_name: peer_name.clone(),
                stored_fingerprint: stored.clone(),
                presented_fingerprint: presented.clone(),
                is_incoming: true,
            });

            let trusted = match timeout(PEER_DECISION_TIMEOUT, decision_rx).await {
                Ok(Ok(v)) => v,
                Ok(Err(_)) => false, // sender dropped
                Err(_) => {
                    pending_decisions.lock().await.remove(&transfer_id);
                    false
                }
            };

            if !trusted {
                return Err(err!(
                    "Fingerprint mismatch rejected by user - aborting transfer"
                ));
            }
        }
    }

    // Build HandshakeResp: send our fingerprint instead of the full public key.
    let ephemeral = EphemeralKeyPair::generate();
    let ephemeral_pub_bytes = ephemeral.public_bytes();
    let our_fp = public_key_fingerprint(&our_public_key)?;

    let resp = HandshakeResp {
        responder_fingerprint: our_fp,
        ephemeral_public: ephemeral_pub_bytes,
    };
    let resp_bytes = tokio::task::spawn_blocking(move || resp.serialize(&our_private_key))
        .await
        .context("RSA sign task panicked")?
        .context("HandshakeResp serialization failed")?;
    match timeout(HANDSHAKE_TIMEOUT, stream.write_all(&resp_bytes)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(err!("Timeout sending handshake response")),
    }

    let password = session_password.lock().expect("Lock poisoned").clone();
    let stretched_password = if !password.is_empty() {
        let salt = init.password_salt;
        tokio::task::spawn_blocking(move || stretch_password(&password, &salt))
            .await
            .context("Password stretch task panicked")?
            .context("Password KDF failed")?
    } else {
        None
    };

    let remote_pub = X25519PublicKey::from(init.ephemeral_public);
    let dh_secret = ephemeral.diffie_hellman(&remote_pub);

    // Responder: initiator_rsa_pub = initiator, our_public_key = responder.
    let key = {
        let init_pub = initiator_rsa_pub.clone();
        tokio::task::spawn_blocking(move || {
            derive_session_key(
                &dh_secret,
                &init_pub,
                &our_public_key,
                stretched_password.as_ref(),
            )
        })
        .await
        .context("Session key derivation task panicked")?
        .context("Session key derivation failed")?
    };

    // Persist the initiator's key to the keystore now that the handshake succeeded.
    if let KeyCheckResult::FirstContact = check_result {
        let peer_key = initiator_rsa_pub.clone();
        let peer_name_store = peer_name.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || trust_peer(&peer_name_store, &peer_key))
            .await
            .context("Key store task panicked")?
        {
            log::warn!("Failed to persist peer key after first contact: {e}");
        }
    }

    Ok(key)
}
