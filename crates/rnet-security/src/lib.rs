//! Transport-independent authenticated session establishment.

pub mod transition;

use curve25519_dalek::montgomery::MontgomeryPoint;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SecurityError {
    #[error("peer key does not match the pinned key")]
    PeerKeyMismatch,
    #[error("invalid handshake state")]
    InvalidState,
    #[error("cryptographic operation failed")]
    Crypto,
    #[error("datagram nonce was already received or is outside the replay window")]
    ReplayDetected,
}

impl From<snow::Error> for SecurityError {
    fn from(_: snow::Error) -> Self {
        Self::Crypto
    }
}

pub type Result<T> = std::result::Result<T, SecurityError>;

#[derive(Clone, Debug, Eq, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct Keypair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

impl Keypair {
    pub fn generate() -> Result<Self> {
        let params = NOISE_PATTERN.parse().map_err(|_| SecurityError::Crypto)?;
        let keypair = snow::Builder::new(params).generate_keypair()?;
        Ok(Self {
            private: keypair.private,
            public: keypair.public,
        })
    }

    pub fn from_private(private: &[u8]) -> Result<Self> {
        let private: [u8; 32] = private.try_into().map_err(|_| SecurityError::Crypto)?;
        let public = MontgomeryPoint::mul_base_clamped(private).to_bytes();
        Ok(Self {
            private: private.to_vec(),
            public: public.to_vec(),
        })
    }
}

pub struct InitiatorHandshake {
    state: snow::HandshakeState,
    expected_peer_public: [u8; 32],
}

pub struct ResponderHandshake {
    state: snow::HandshakeState,
}
pub struct SecureTransport {
    state: snow::TransportState,
}
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
        })
    }

    pub fn write_first(&mut self) -> Result<Vec<u8>> {
        write_handshake_message(&mut self.state, &[])
    }

    pub fn read_response(&mut self, message: &[u8]) -> Result<()> {
        read_handshake_message(&mut self.state, message)?;
        let peer = self
            .state
            .get_remote_static()
            .ok_or(SecurityError::InvalidState)?;
        if peer != self.expected_peer_public {
            return Err(SecurityError::PeerKeyMismatch);
        }
        Ok(())
    }

    pub fn finish(mut self, join_payload: &[u8]) -> Result<(Vec<u8>, SecureTransport)> {
        let message = write_handshake_message(&mut self.state, join_payload)?;
        let state = self.state.into_transport_mode()?;
        Ok((message, SecureTransport { state }))
    }

    pub fn finish_datagram(mut self, join_payload: &[u8]) -> Result<(Vec<u8>, DatagramTransport)> {
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

    pub fn read_first(&mut self, message: &[u8]) -> Result<()> {
        read_handshake_message(&mut self.state, message).map(|_| ())
    }

    pub fn write_response(&mut self) -> Result<Vec<u8>> {
        write_handshake_message(&mut self.state, &[])
    }

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

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut ciphertext = vec![0; plaintext.len() + 16];
        let written = self.state.write_message(plaintext, &mut ciphertext)?;
        ciphertext.truncate(written);
        Ok(ciphertext)
    }

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

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce = self.send_nonce;
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
