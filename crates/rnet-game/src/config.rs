//! Public game-facing configuration.

use rnet_core::Transport;
use rnet_security::Keypair;
use rnet_transport::RuntimeConfig;
use std::net::SocketAddr;

/// Exact application protocol identity selected by one client connection.
///
/// Version zero and protocol ID zero are reserved. The server accepts the client only when both
/// values match its listener contract; build and capability fields are authenticated join
/// metadata for application policy, not permission to enable library features.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameProtocol {
    pub protocol_id: u64,
    pub version: u32,
    pub build_id: u64,
    pub capabilities: u64,
}

impl GameProtocol {
    pub const fn new(protocol_id: u64, version: u32) -> Self {
        Self {
            protocol_id,
            version,
            build_id: 0,
            capabilities: 0,
        }
    }

    pub(crate) fn is_valid(self) -> bool {
        self.protocol_id != 0 && self.version != 0
    }
}

/// Runtime limits and policies used by the game facade.
#[derive(Clone, Debug)]
pub struct GameRuntimeConfig {
    /// Low-level capacity, timeout, socket, and security policy.
    pub network: RuntimeConfig,
}

impl GameRuntimeConfig {
    /// Secure defaults for a new deployment.
    pub fn production() -> Self {
        Self {
            network: RuntimeConfig::production(),
        }
    }

    /// Explicitly permits a server to switch business records to plaintext.
    ///
    /// This does not enable legacy unauthenticated endpoints. Noise setup and security-control
    /// messages stay authenticated; only subsequent business payloads lose confidentiality and
    /// integrity while the server-selected plaintext mode is active.
    pub fn allow_plaintext_business_data(mut self, allow: bool) -> Self {
        self.network.security_policy.allow_plaintext_business_data = allow;
        self
    }

    /// Allows explicit plaintext business-data transitions for migrations and controlled tests.
    pub fn compatibility() -> Self {
        Self {
            network: RuntimeConfig::default(),
        }
    }
}

impl Default for GameRuntimeConfig {
    fn default() -> Self {
        Self::production()
    }
}

/// Server listener configuration; transport and initial encryption are fixed before accepting.
#[derive(Clone, Debug)]
pub struct GameServerConfig {
    pub transport: Transport,
    pub bind_addr: SocketAddr,
    pub local_key: Keypair,
    /// Initial business-data protection. The server may change it after authorization.
    pub initial_encryption: bool,
    /// Exact protocol identity and version required before application authorization.
    pub protocol: GameProtocol,
}

/// Client join configuration; encryption is intentionally controlled by the server.
#[derive(Clone, Debug)]
pub struct GameClientConfig {
    pub transport: Transport,
    pub bind_addr: Option<SocketAddr>,
    pub remote_addr: SocketAddr,
    /// Opaque application ticket delivered only after the cryptographic handshake.
    pub join_ticket: Vec<u8>,
    pub protocol: GameProtocol,
}

/// Domain-name client join; resolution and cross-address-family retries use the transport layer.
#[derive(Clone, Debug)]
pub struct GameHostClientConfig {
    pub transport: Transport,
    pub host: String,
    pub port: u16,
    pub join_ticket: Vec<u8>,
    pub protocol: GameProtocol,
}
