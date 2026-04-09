# Crypto protocol

## Crypto protocol details

### Static identity (generated once per device, persisted to disk)

```
  +----------------------------+
  |  RSA-4096 Key Pair         |
  |  (PKCS#8 DER, mode 0600)   |
  |                            |
  |  priv_A  <-->  pub_A       |  Peer A
  +----------------------------+

  +----------------------------+
  |  RSA-4096 Key Pair         |
  |                            |
  |  priv_B  <-->  pub_B       |  Peer B
  +----------------------------+

  Fingerprint for peer identity / display:
    fp_A = SHA3-256( DER(pub_A) )    (32 bytes, shown as hex with colons)
    fp_B = SHA3-256( DER(pub_B) )
```

### Phase 1 - Discovery  (UDP broadcast, clear text)

Peer A broadcasts every ~1 s on UDP port 42300:

```
  DiscoveryPacket (628 bytes, manual serialization):
  +----------+----+------+-------+-------------------+---------+
  | name(64) |nlen| port | klen  |  pub_A DER (550)  |  CRC64  |
  |          |(2) | (2)  |  (2)  |  zero-padded      |   (8)   |
  +----------+----+------+-------+-------------------+---------+
    0        64  66     68      70                  620       628
```

- Peer B receives the packet, computes fp_A from pub_A, and stores the device.
- Peer A receives Peer B's packet and stores it under fp_B.
- Keys are NOT yet written to the on-disk keystore.

### Phase 2 - Handshake  (TCP port 42301, signed clear text)

Goal: mutually authenticate, exchange ephemeral DH keys, derive session key.
Both handshake packets are transmitted in clear text and are authenticated with
an RSA-PSS signature. Neither x25519 public keys nor Argon2 salts are sensitive,
so encryption of the handshake provides no security benefit.

The full RSA public key DER is not included in the handshake packets.
Each packet carries only the 32-byte SHA3-256 fingerprint of the sender's key.
Before verifying any packet signature, the receiver must locate the sender's
public key using that fingerprint:
  1. Check the on-disk keystore first (persistent, verified in a prior session).
  2. If not in the keystore, check the in-memory discovery map (current broadcast).
  3. If found in neither, abort the handshake.

```
  Peer A (initiator/sender) connects to Peer B (responder/receiver).

  Step 1 - Initiator generates ephemeral material:
    eph_priv_A, eph_pub_A  =  x25519_keygen()       (32 bytes each)
    pw_salt                =  random(32 bytes)

  Step 2 - Initiator sends HandshakeInit (608 bytes):

    Wire format:
    +-------------------+-------------------+------------------+------------------------------------------+
    |  fp_A       (32)  |  eph_pub_A  (32)  |  pw_salt  (32)   |  RSA-PSS-SHA3-256 signature (512 bytes)  |
    |  SHA3-256(pub_A)  |                   |                  |  over bytes [0..96], signed by priv_A    |
    +-------------------+-------------------+------------------+------------------------------------------+

    fp_A identifies the initiator. Before verifying the signature, the responder
    looks up pub_A (keystore first, then in-memory discovery map). If the key
    cannot be found, the handshake is aborted.

         A                                             B
         |  HandshakeInit (608 bytes)                  |
         |-------------------------------------------->|
         |                                             |  read fp_A from wire
         |                                             |  look up pub_A by fp_A
         |                                             |  RSA-PSS verify(pub_A, bytes[0..96], sig)
         |                                             |  extract: eph_pub_A, pw_salt
         |                                             |  verify peer key(fp_A, pub_A)
         |                                             |  eph_priv_B, eph_pub_B = x25519_keygen()

  Step 3 - Responder sends HandshakeResp (576 bytes):

    Wire format:
    +-------------------+-------------------+------------------------------------------+
    |  fp_B       (32)  |  eph_pub_B  (32)  |  RSA-PSS-SHA3-256 signature (512 bytes)  |
    |  SHA3-256(pub_B)  |                   |  over bytes [0..64], signed by priv_B    |
    +-------------------+-------------------+------------------------------------------+

    fp_B allows the initiator to verify that the responder is the expected peer.
    Before verifying the signature, the initiator looks up pub_B (keystore first,
    then falls back to the key from discovery already held in memory).

         |                                             |
         |  HandshakeResp (576 bytes)                  |
         |<--------------------------------------------|
         |  read fp_B from wire                        |
         |  verify fp_B == fingerprint(expected peer)  |
         |  look up pub_B by fp_B (keystore preferred) |
         |  RSA-PSS verify(pub_B, bytes[0..64], sig)   |
         |  extract: eph_pub_B                         |
         |  verify peer key(fp_B, pub_B)               |
```

### Phase 3 - Session key derivation  (both peers, independently, same result)

```
  Step 4 - DH shared secret:
    dh_secret = x25519(eph_priv_A, eph_pub_B)          (Peer A)
              = x25519(eph_priv_B, eph_pub_A)          (Peer B, same value)
    (32 bytes)

  Step 5 - Optional password stretching (if user set a password):
    pw_material = Argon2id(
        password  = user_password,
        salt      = pw_salt,                   (from HandshakeInit)
        m         = 65536 KiB  (~64 MiB),
        t         = 3 iterations,
        p         = 4 threads,
        out_len   = 32 bytes
    )
    Skipped (pw_material = None) if password is empty.

  Step 6 - Session key mixing:
    kdf_input  = dh_secret || DER(pub_A) || DER(pub_B) || pw_material
                 (pw_material is 32 zero bytes if no password was set)

    session_key = Argon2id(
        password  = kdf_input,                              (all secrets combined)
        salt      = "ch.bues.transfer.session.key.v1",      (constant domain-separation string)
        m         = 128 KiB,
        t         = 1 iteration,
        p         = 1 thread,
        out_len   = 32 bytes
    )

  Both peers independently compute the identical 32-byte session_key.

  The session key binds:
    - The ephemeral DH secret           (forward secrecy)
    - Both static RSA public keys       (identity binding)
    - The optional user password        (additional secret)

  Step 7 - Keystore update:
    If peer is new (first contact):
      persist fingerprint + pub_key to on-disk keystore (~/.local/share/ch.bues.transfer/)
      keyed by fp_X (fingerprint hex) - not by UUID
    If stored key fingerprint does not match:
      emit KeyMismatchWarning event; wait for user decision; persist only if accepted.
```

### Phase 4 - Encrypted data transfer  (TCP, AES-256-GCM)

Every application-level message (TransferHeader, file chunks, ACCEPT/REJECT)
is wrapped in an EncryptedPacket:

```
  Wire layout:

  v  EncryptedPacketHeader  (24 bytes)   v  AES-256-GCM ciphertext  v
  |  (AAD - authenticated, not enc.)     |  + 16-byte auth tag      |
  +--------+-----------+-----------------+--------------------------+
  | ts(8)  | nonce(12) | len(4)          |  ciphertext + auth tag   |
  +--------+-----------+-----------------+--------------------------+

  Encryption:
    nonce       = random(12 bytes), fresh per message
    ciphertext  = AES-256-GCM-Enc(
                      key   = session_key,
                      nonce = nonce,
                      aad   = header_bytes,    <- authenticates header
                      msg   = plaintext
                  )
    on-wire     = header_bytes(24) || ciphertext || auth_tag(16)
                  (len is inside header_bytes, authenticated as AAD)

  Decryption and verification (receiver):
    1. Read and parse EncryptedPacketHeader (24 bytes); extract ciphertext_len.
    2. Verify timestamp is within +/- 5 minutes of local clock.
    3. Verify timestamp > last_received_timestamp_from_this_peer
       (monotonically increasing - prevents replay within the tolerance window).
    4. AES-256-GCM-Dec; if auth tag fails, abort session.

  File transfer message sequence:
                  Initiator (sender)                  Responder (receiver)
                       |                                    |
                       |                 HandshakeInit -->  |
                       |  <-- HandshakeResp                 |
    derive session_key |                                    |  derive (same) session_key
                       |                                    |
                       |           Enc(TransferHeader) -->  |
                       |  <-- Enc(ACCEPT) or Enc(REJECT)    |
                       |              Enc(zip chunk 1) -->  |
                       |              Enc(zip chunk 2) -->  |
                       |              ...                   |
                       |              Enc(zip chunk N) -->  |
                       |        [close]                     |
```

### Algorithms and parameters summary

| Purpose              | Algorithm              | Parameters                  |
| -------------------- | ---------------------- | --------------------------- |
| Static identity      | RSA-4096               | PKCS#8 DER                  |
| Handshake sign       | RSA-PSS-SHA3-256       | 512-byte signature          |
| DH key exchange      | x25519                 | 32-byte shared secret       |
| Password KDF         | Argon2id               | m=64 MiB, t=3, p=4          |
| Session key mix      | Argon2id               | m=128 KiB, t=1, p=1         |
| Symmetric encrypt    | AES-256-GCM            | 32-byte key, 12-byte nonce  |
| Identity fingerprint | SHA3-256(DER(pub))     | 32 bytes, displayed as hex  |
| Packet integrity     | CRC                    | CRC64-NVME                  |
| Key persistence      | serde_json + base64    | On-disk keystore            |

## Implementation notes

### Secrets

- Each peer shall have a long-term static RSA public/private key pair for authentication.
- An ephemeral secret key shall be negotiated (DH) for each session and used for encryption.
- The ephemeral secret keys shall be used for a single session and discarded afterwards.
- Optionally, a user-provided password may be used as an additional secret for key derivation.
  The user-provided password shall be converted to a fixed-length secret using the `Argon2id` key derivation function.
  The parameters for the KDF shall be: Medium duration multi threaded execution (~500 ms) and medium memory usage (~64 MiB).

### Handshake

- A handshake shall be performed at the beginning of each session to authenticate the peers and establish a shared session key for encryption.
- The handshake request and response packets shall be transmitted in clear text.
  Neither x25519 public keys nor Argon2 salts are sensitive, so no encryption is needed.
- The handshake request shall be signed with the initiator's static RSA private key.
  The responder shall verify the signature before processing the packet contents.
- The handshake response shall be signed with the responder's static RSA private key.
  The initiator shall verify the signature before processing the packet contents.
- Handshake packets shall include only the 32-byte SHA3-256 fingerprint of the sender's
  RSA public key, not the full DER-encoded key.
  The receiver must retrieve the sender's public key from the keystore (preferred) or
  from the in-memory discovery map before it can verify the RSA-PSS signature.
  If the key cannot be found in either store, the handshake shall be aborted.

### Public key exchange

- The RSA public key shall be included in the discovery packets.
- Received public keys from discovery packets shall be stored in memory first.
- Public keys shall be written persistently to the on-disk keystore only after a successful handshake with the peer.
- When looking up a peer's public key during the handshake, the keystore shall always be checked first
  and preferred over the in-memory discovery copy.

### DH key exchange

- The peers shall perform a `x25519` Diffie-Hellman (DH) key exchange to exchange the session key for encryption.

### Peer Identity Verification

- Each peer shall store peer's public key on first approved connection to the local keystore.
- On subsequent connections, the peer shall verify that the public key matches the stored value.
- If the public key does not match, the peer shall reject the connection and log a warning about a potential man-in-the-middle attack.
- If a new peer is encountered, the user shall be prompted to accept the new peer.

### Encrypted packet format for data transfer

- Each encrypted packet shall consist of a header and a payload.
- The payload shall be the `TransferHeader` and its corresponding data.
  - The header shall contain metadata such as a timestamp (microsecond granularity) and a nonce for encryption (96 bit).
- The header shall be authenticated but not encrypted.
- An authentication tag for integrity verification shall be appended to the end of the packet.

### Packet verification

- Upon receiving a packet, the peer shall verify the authentication tag using the session key.
- The peer shall verify that the timestamp is within an acceptable time window to prevent replay attacks.
  Time stamps shall be monotonically increasing for subsequent packets from the same peer.
  Time stamps shall be verified against the local clock with a tolerance of 5 minutes.
- If the packet fails verification, the peer shall discard it, log an error and abort the session.

### Encrypted communication and clear text communication

- All packets related to file transfers shall use encrypted communication.
- All packets related to peer discovery and connection establishment shall use clear text communication.

### GUI

The GUI shall:

- Query the user for a password on startup and use it as an additional secret for key derivation.
  This password shall be an option and can be left empty if the user does not want to use it.
  It shall not be in the main usage flow and should be clearly marked as an optional security enhancement.
- Display a warning if the user tries to connect to a peer whose static key hash does not match the stored value, indicating a potential man-in-the-middle attack.
- Provide an option to override the warning and proceed with the connection, but only after the user explicitly confirms that they understand the risks.
- All in all the encryption features should be designed to be as unobtrusive as possible.
  The default usage flow should work seamlessly without requiring the user to interact with the encryption features, while still providing basic security through the use of static keys and explicit peer approval.

### Crates

- Crate `x25519-dalek` for DH key exchange.
- Crate `rsa` for RSA key generation and encryption.
- Crate `aes-gcm` for authenticated encryption.
- Crate `sha3` for hashing.
- Crate `getrandom` for secure random number generation.
- Crate `argon2` for password-based key derivation.
- Crate `rkyv` for serialization and deserialization of packets.
- Crate `serde` and `serde_json` for serialization and deserialization of **keystore** data structures.
- Crate `base64` for base64 encoding and decoding.
