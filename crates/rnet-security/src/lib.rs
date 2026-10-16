//! Transport-independent Noise XX establishment, ordered/datagram protection and rekey barriers.
//!
//! A successful handshake proves identity-key possession, not account authorization.
//! The client must trust its server pin out of band; the server must authorize returned
//! join credentials. Bound record sizes at the caller and never log secret payloads.

#![deny(missing_docs)]

pub mod transition;

use curve25519_dalek::montgomery::MontgomeryPoint;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

/// Security-layer failures; underlying cipher errors intentionally omit low-level detail.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum SecurityError {
    /// Missing/incorrect pinned server public key, or a presented identity that does not match it.
    #[error("peer key does not match the pinned key")]
    PeerKeyMismatch,
    /// Missing verified handshake prerequisite or exhausted datagram nonce space.
    #[error("invalid handshake state")]
    InvalidState,
    /// Malformed key/message, authentication failure or another Noise operation failure.
    #[error("cryptographic operation failed")]
    Crypto,
    /// Repeated datagram nonce or nonce outside the 64-position receive window.
    #[error("datagram nonce was already received or is outside the replay window")]
    ReplayDetected,
}

impl From<snow::Error> for SecurityError {
    fn from(_: snow::Error) -> Self {
        Self::Crypto
    }
}

/// Result of a cryptographic operation; not a transport I/O or business authorization result.
pub type Result<T> = std::result::Result<T, SecurityError>;

/// Owned X25519 identity; both vectors are zeroized on drop, including independent clones.
/// Debug redacts the private bytes only. Public fields permit malformed lengths; use constructors.
#[derive(Clone, Eq, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct Keypair {
    /// Exactly 32 secret bytes from constructors; never log or distribute them.
    /// Cloning/extracting bytes creates additional caller-owned secret copies.
    pub private: Vec<u8>,
    /// Corresponding 32-byte identity key for trusted pin distribution; not an encryption-policy flag.
    pub public: Vec<u8>,
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Keypair")
            .field("private", &"<redacted>")
            .field("public", &self.public)
            .finish()
    }
}

impl Keypair {
    /// Generates an identity through the Noise cryptographic RNG; failures return Crypto.
    pub fn generate() -> Result<Self> {
        let params = NOISE_PATTERN.parse().map_err(|_| SecurityError::Crypto)?;
        let keypair = snow::Builder::new(params).generate_keypair()?;
        Ok(Self {
            private: keypair.private,
            public: keypair.public,
        })
    }

    /// Copies exactly 32 private bytes and derives the public key; other lengths return Crypto.
    /// The borrowed caller input is not erased; this is identity restoration, not key rotation.
    pub fn from_private(private: &[u8]) -> Result<Self> {
        let private: [u8; 32] = private.try_into().map_err(|_| SecurityError::Crypto)?;
        let public = MontgomeryPoint::mul_base_clamped(private).to_bytes();
        Ok(Self {
            private: private.to_vec(),
            public: public.to_vec(),
        })
    }
}

/// Pinned-server Noise XX client handshake: write_first, read_response, then one finish method.
/// Finish requires successful pin verification, not merely a progressed Noise state.
/// Discard on error; handshake completion is not server application authorization.
pub struct InitiatorHandshake {
    state: snow::HandshakeState,
    expected_peer_public: [u8; 32],
    // Noise advances before the external pin check; its state alone is not trust evidence.
    peer_verified: bool,
}

/// Noise XX server handshake: read_first, write_response, then one finish method.
/// Completion proves the client's key possession, not account permission: authorize the
/// returned identity and join payload separately. Discard the handshake on error.
pub struct ResponderHandshake {
    state: snow::HandshakeState,
}
/// Stateful authenticated records for reliable ordered delivery (TCP or ordered KCP).
/// No framing or application authorization is provided; preserve record order and boundaries.
/// Coordinate rekeys through the authenticated transition protocol, not ad hoc local calls.
pub struct SecureTransport {
    state: snow::TransportState,
}
/// Authenticated datagrams with explicit nonces and a 64-position replay window.
/// Allows unseen reordered records within that window, not reliable delivery.
/// The embedding driver must coordinate key epochs and bound input sizes before allocation.
pub struct DatagramTransport {
    state: snow::StatelessTransportState,
    send_nonce: u64,
    replay: ReplayWindow,
}

#[derive(Default)]
struct ReplayWindow {
    highest: Option<u64>,
    bitmap: u64,
}

impl InitiatorHandshake {
    /// Creates a client using a 32-byte local identity and a trusted 32-byte server pin.
    /// An invalid pin length returns PeerKeyMismatch; an invalid private key returns Crypto.
    /// Inputs are copied into handshake state; callers retain responsibility for their copies.
    pub fn new(local_private: &[u8], expected_peer_public: &[u8]) -> Result<Self> {
        if expected_peer_public.len() != 32 {
            return Err(SecurityError::PeerKeyMismatch);
        }
        if local_private.len() != 32 {
            return Err(SecurityError::Crypto);
        }
        let params = NOISE_PATTERN.parse().map_err(|_| SecurityError::Crypto)?;
        let state = snow::Builder::new(params)
            .local_private_key(local_private)?
            .build_initiator()?;
        let mut expected = [0; 32];
        expected.copy_from_slice(expected_peer_public);
        Ok(Self {
            state,
            expected_peer_public: expected,
            peer_verified: false,
        })
    }

    /// Emits the first Noise message with no application payload; call once on a fresh handshake.
    /// The returned bytes need transport framing/delivery. Invalid Noise sequencing returns Crypto.
    pub fn write_first(&mut self) -> Result<Vec<u8>> {
        write_handshake_message(&mut self.state, &[])
    }

    /// Authenticates the complete second Noise message against the constructor's server pin.
    /// Mismatch returns PeerKeyMismatch; malformed/out-of-sequence data returns Crypto.
    /// Any failed response invalidates finish permission; never send credentials after an error.
    pub fn read_response(&mut self, message: &[u8]) -> Result<()> {
        self.peer_verified = false;
        read_handshake_message(&mut self.state, message)?;
        let peer = self
            .state
            .get_remote_static()
            .ok_or(SecurityError::InvalidState)?;
        if peer != self.expected_peer_public {
            return Err(SecurityError::PeerKeyMismatch);
        }
        self.peer_verified = true;
        Ok(())
    }

    /// Consumes a pin-verified handshake and returns (final protected handshake bytes, ordered cipher).
    /// `join_payload` is opaque credential data, possibly empty; this method does not authorize it.
    /// Missing pin verification returns InvalidState. Noise length/state failures return Crypto.
    /// Deliver the final message before cipher records; await server authorization at the next layer.
    pub fn finish(mut self, join_payload: &[u8]) -> Result<(Vec<u8>, SecureTransport)> {
        if !self.peer_verified {
            return Err(SecurityError::InvalidState);
        }
        let message = write_handshake_message(&mut self.state, join_payload)?;
        let state = self.state.into_transport_mode()?;
        Ok((message, SecureTransport { state }))
    }

    /// Datagram counterpart of finish: returns final handshake bytes plus a fresh nonce/replay state.
    /// Requires a successfully verified response (InvalidState otherwise); Noise failures return Crypto.
    /// Join bytes are protected but not authorized; the caller must complete the server decision flow.
    pub fn finish_datagram(mut self, join_payload: &[u8]) -> Result<(Vec<u8>, DatagramTransport)> {
        if !self.peer_verified {
            return Err(SecurityError::InvalidState);
        }
        let message = write_handshake_message(&mut self.state, join_payload)?;
        let state = self.state.into_stateless_transport_mode()?;
        Ok((
            message,
            DatagramTransport {
                state,
                send_nonce: 0,
                replay: ReplayWindow::default(),
            },
        ))
    }
}

impl ResponderHandshake {
    /// Creates a server from exactly 32 private identity bytes; invalid length/setup returns Crypto.
    /// The caller retains its input bytes and must erase its own secret copies when appropriate.
    pub fn new(local_private: &[u8]) -> Result<Self> {
        if local_private.len() != 32 {
            return Err(SecurityError::Crypto);
        }
        let params = NOISE_PATTERN.parse().map_err(|_| SecurityError::Crypto)?;
        let state = snow::Builder::new(params)
            .local_private_key(local_private)?
            .build_responder()?;
        Ok(Self { state })
    }

    /// Consumes the first complete client Noise message; it does not yet authenticate a client identity.
    /// Malformed data or invalid message order returns Crypto.
    pub fn read_first(&mut self, message: &[u8]) -> Result<()> {
        read_handshake_message(&mut self.state, message).map(|_| ())
    }

    /// Emits the second Noise message after read_first; the client must check its server pin.
    /// Returned bytes are caller-owned and need transport framing; sequencing errors return Crypto.
    pub fn write_response(&mut self) -> Result<Vec<u8>> {
        write_handshake_message(&mut self.state, &[])
    }

    /// Consumes the final client message and returns (join bytes, authenticated client key, ordered cipher).
    /// Malformed/unauthentic messages fail; no cipher is returned. Business authorization is
    /// still required before accepting application data. Caller owns and must protect join bytes.
    pub fn finish(mut self, message: &[u8]) -> Result<(Vec<u8>, [u8; 32], SecureTransport)> {
        let payload = read_handshake_message(&mut self.state, message)?;
        let peer = self
            .state
            .get_remote_static()
            .ok_or(SecurityError::InvalidState)?;
        let mut peer_key = [0; 32];
        peer_key.copy_from_slice(peer);
        let state = self.state.into_transport_mode()?;
        Ok((payload, peer_key, SecureTransport { state }))
    }

    /// Consumes the final client message and returns (join bytes, client key, datagram cipher).
    /// Uses fresh nonce/replay state instead of ordered implicit nonces. Authentication failures
    /// return Crypto; the returned client identity and credentials still need business authorization.
    pub fn finish_datagram(
        mut self,
        message: &[u8],
    ) -> Result<(Vec<u8>, [u8; 32], DatagramTransport)> {
        let payload = read_handshake_message(&mut self.state, message)?;
        let peer = self
            .state
            .get_remote_static()
            .ok_or(SecurityError::InvalidState)?;
        let mut peer_key = [0; 32];
        peer_key.copy_from_slice(peer);
        let state = self.state.into_stateless_transport_mode()?;
        Ok((
            payload,
            peer_key,
            DatagramTransport {
                state,
                send_nonce: 0,
                replay: ReplayWindow::default(),
            },
        ))
    }
}

impl SecureTransport {
    /// Derives the next outgoing transport key without resetting the Noise nonce.
    pub fn rekey_outgoing(&mut self) {
        self.state.rekey_outgoing();
    }

    /// Derives the next incoming transport key without resetting the Noise nonce.
    pub fn rekey_incoming(&mut self) {
        self.state.rekey_incoming();
    }

    /// Encrypts one complete record with the next implicit send nonce; returns owned ciphertext/tag.
    /// Empty plaintext is valid. Noise limits/auth-state failures return Crypto. Bound caller input
    /// before allocation; preserve successful record order and perform transport framing separately.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut ciphertext = vec![0; plaintext.len() + 16];
        let written = self.state.write_message(plaintext, &mut ciphertext)?;
        ciphertext.truncate(written);
        Ok(ciphertext)
    }

    /// Authenticates/decrypts exactly the next ordered record; returns owned plaintext on success.
    /// Modified, truncated or out-of-sequence records return Crypto; never use failed plaintext.
    /// The caller must bound ciphertext length before the temporary plaintext allocation.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut plaintext = vec![0; ciphertext.len()];
        let written = self.state.read_message(ciphertext, &mut plaintext)?;
        plaintext.truncate(written);
        Ok(plaintext)
    }
}

impl DatagramTransport {
    /// Derives the next outgoing key and starts a fresh nonce space for the new epoch.
    pub fn rekey_outgoing(&mut self) {
        self.state.rekey_outgoing();
        self.send_nonce = 0;
    }

    /// Derives the next incoming key and resets replay tracking for the new epoch.
    pub fn rekey_incoming(&mut self) {
        self.state.rekey_incoming();
        self.replay = ReplayWindow::default();
    }

    /// Encrypts one datagram; output is an 8-byte big-endian nonce followed by ciphertext/tag.
    /// The nonce is reserved before encryption and is not reused even on failure. Nonce exhaustion
    /// returns InvalidState; cipher/size failures return Crypto. Bound input before allocation.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = self.send_nonce;
        // A failed encryption burns the reserved nonce; reuse would violate the
        // cipher's per-key uniqueness contract even if no packet was emitted.
        self.send_nonce = self
            .send_nonce
            .checked_add(1)
            .ok_or(SecurityError::InvalidState)?;
        let mut record = vec![0; 8 + plaintext.len() + 16];
        record[..8].copy_from_slice(&nonce.to_be_bytes());
        let written = self
            .state
            .write_message(nonce, plaintext, &mut record[8..])?;
        record.truncate(8 + written);
        Ok(record)
    }

    /// Authenticates a nonce-prefixed datagram and copies its plaintext on success.
    /// Duplicates/too-old nonces return ReplayDetected; truncation or failed tags return Crypto.
    /// Only authenticated records advance the replay window, so forged high nonces cannot
    /// poison it. Bound input size before allocation; loss and reordering remain possible.
    pub fn decrypt(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if record.len() < 24 {
            return Err(SecurityError::Crypto);
        }
        let nonce = u64::from_be_bytes(record[..8].try_into().map_err(|_| SecurityError::Crypto)?);
        if self.replay.contains(nonce) {
            return Err(SecurityError::ReplayDetected);
        }
        let mut plaintext = vec![0; record.len() - 8];
        let written = self
            .state
            .read_message(nonce, &record[8..], &mut plaintext)?;
        // Authentication must precede replay-window mutation: a forged high nonce
        // must not evict valid in-flight datagrams from the 64-position window.
        self.replay.insert(nonce)?;
        plaintext.truncate(written);
        Ok(plaintext)
    }
}

impl ReplayWindow {
    fn contains(&self, nonce: u64) -> bool {
        let Some(highest) = self.highest else {
            return false;
        };
        if nonce > highest {
            return false;
        }
        let distance = highest - nonce;
        distance >= 64 || self.bitmap & (1_u64 << distance) != 0
    }

    fn insert(&mut self, nonce: u64) -> Result<()> {
        match self.highest {
            None => {
                self.highest = Some(nonce);
                self.bitmap = 1;
            }
            Some(highest) if nonce > highest => {
                let distance = nonce - highest;
                self.bitmap = if distance >= 64 {
                    1
                } else {
                    (self.bitmap << distance) | 1
                };
                self.highest = Some(nonce);
            }
            Some(highest) => {
                let distance = highest - nonce;
                if distance >= 64 || self.bitmap & (1_u64 << distance) != 0 {
                    return Err(SecurityError::ReplayDetected);
                }
                self.bitmap |= 1_u64 << distance;
            }
        }
        Ok(())
    }
}

fn write_handshake_message(state: &mut snow::HandshakeState, payload: &[u8]) -> Result<Vec<u8>> {
    let mut message = vec![0; payload.len() + 1024];
    let written = state.write_message(payload, &mut message)?;
    message.truncate(written);
    Ok(message)
}

fn read_handshake_message(state: &mut snow::HandshakeState, message: &[u8]) -> Result<Vec<u8>> {
    let mut payload = vec![0; message.len()];
    let written = state.read_message(message, &mut payload)?;
    payload.truncate(written);
    Ok(payload)
}
