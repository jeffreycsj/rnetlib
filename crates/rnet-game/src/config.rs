//! Public game-facing configuration.

use crate::quality::QualityPolicy;
use crate::realtime::RealtimeQueueConfig;
use crate::scheduler::ScheduledQueueConfig;
use rnet_core::{ErrorCode, RnetError, Transport};
use rnet_security::Keypair;
use rnet_transport::RuntimeConfig;
use std::net::SocketAddr;
use std::time::Duration;

/// High-level game traffic shape used only to choose safe connection defaults.
///
/// A profile is expanded into one immutable transport when configuration is created. It is not
/// sent on the wire and never appears in the message send/receive API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GameProfile {
    /// Loss-tolerant input/state traffic: native UDP.
    Realtime,
    /// Reliable, ordered, latency-sensitive traffic: KCP.
    ReliableRealtime,
    /// Reliable session/login/turn traffic: TCP.
    Session,
}

impl GameProfile {
    /// Returns the stable recommended transport for this profile.
    pub const fn transport(self) -> Transport {
        match self {
            Self::Realtime => Transport::Udp,
            Self::ReliableRealtime => Transport::Kcp,
            Self::Session => Transport::Tcp,
        }
    }
}

impl TryFrom<u32> for GameProfile {
    type Error = RnetError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Realtime),
            2 => Ok(Self::ReliableRealtime),
            3 => Ok(Self::Session),
            _ => Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "unknown game profile",
            )),
        }
    }
}

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

/// Protocol identity and inclusive version interval for an explicit wire-v4 join.
///
/// Build and capability values remain authenticated application metadata. They do not grant
/// library capabilities. A range is valid only when its ID and both bounds are nonzero and the
/// lower bound does not exceed the upper bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameProtocolRange {
    pub protocol_id: u64,
    pub min_version: u32,
    pub max_version: u32,
    pub build_id: u64,
    pub capabilities: u64,
}

impl GameProtocolRange {
    pub const fn new(protocol_id: u64, min_version: u32, max_version: u32) -> Self {
        Self {
            protocol_id,
            min_version,
            max_version,
            build_id: 0,
            capabilities: 0,
        }
    }

    pub(crate) fn is_valid(self) -> bool {
        self.protocol_id != 0 && self.min_version != 0 && self.min_version <= self.max_version
    }
}

/// Runtime limits and policies used by the game facade.
#[derive(Clone, Debug)]
pub struct GameRuntimeConfig {
    /// Low-level capacity, timeout, socket, and security policy.
    pub network: RuntimeConfig,
    /// Interval between authenticated probes after a successful acknowledgement.
    pub heartbeat_interval: Duration,
    /// Maximum time to wait for a matching probe acknowledgement.
    pub heartbeat_timeout: Duration,
    /// Thresholds for RTT/jitter and, when available, UDP sequence-gap classification.
    pub quality_policy: QualityPolicy,
    /// Independent bounded staging budget for coalesced realtime state messages.
    pub realtime_queue: RealtimeQueueConfig,
    /// Bounded staging for advanced priority/expiry sends.
    pub scheduled_queue: ScheduledQueueConfig,
    /// Lifetime of a one-use, client-key-bound resume credential.
    pub resume_ticket_ttl: Duration,
    /// Maximum outstanding credentials in this runtime; state is not shared across processes.
    pub max_resume_tickets: usize,
}

impl GameRuntimeConfig {
    /// Secure defaults for a new deployment.
    pub fn production() -> Self {
        Self {
            network: RuntimeConfig::production(),
            heartbeat_interval: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(15),
            quality_policy: QualityPolicy::default(),
            realtime_queue: RealtimeQueueConfig::default(),
            scheduled_queue: ScheduledQueueConfig::default(),
            resume_ticket_ttl: Duration::from_secs(30),
            max_resume_tickets: 65_536,
        }
    }

    /// Tunes automatic heartbeat cadence without changing the normal send API.
    pub fn with_heartbeat(mut self, interval: Duration, timeout: Duration) -> Self {
        self.heartbeat_interval = interval;
        self.heartbeat_timeout = timeout;
        self
    }

    pub fn with_quality_policy(mut self, policy: QualityPolicy) -> Self {
        self.quality_policy = policy;
        self
    }

    pub fn with_realtime_queue(mut self, queue: RealtimeQueueConfig) -> Self {
        self.realtime_queue = queue;
        self
    }

    pub fn with_scheduled_queue(mut self, queue: ScheduledQueueConfig) -> Self {
        self.scheduled_queue = queue;
        self
    }

    pub fn with_resume(mut self, ttl: Duration, max_tickets: usize) -> Self {
        self.resume_ticket_ttl = ttl;
        self.max_resume_tickets = max_tickets;
        self
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
            heartbeat_interval: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(15),
            quality_policy: QualityPolicy::default(),
            realtime_queue: RealtimeQueueConfig::default(),
            scheduled_queue: ScheduledQueueConfig::default(),
            resume_ticket_ttl: Duration::from_secs(30),
            max_resume_tickets: 65_536,
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

impl GameServerConfig {
    /// Creates an encrypted exact-version listener using the profile's recommended transport.
    pub fn for_profile(
        profile: GameProfile,
        bind_addr: SocketAddr,
        local_key: Keypair,
        protocol: GameProtocol,
    ) -> Self {
        Self {
            transport: profile.transport(),
            bind_addr,
            local_key,
            initial_encryption: true,
            protocol,
        }
    }
}

/// Explicit wire-v4 listener. Existing `GameServerConfig` remains exact-version wire v3.
#[derive(Clone, Debug)]
pub struct GameRangeServerConfig {
    pub transport: Transport,
    pub bind_addr: SocketAddr,
    pub local_key: Keypair,
    pub initial_encryption: bool,
    pub protocol: GameProtocolRange,
}

impl GameRangeServerConfig {
    /// Creates an encrypted wire-v4 listener using the profile's recommended transport.
    pub fn for_profile(
        profile: GameProfile,
        bind_addr: SocketAddr,
        local_key: Keypair,
        protocol: GameProtocolRange,
    ) -> Self {
        Self {
            transport: profile.transport(),
            bind_addr,
            local_key,
            initial_encryption: true,
            protocol,
        }
    }
}

/// Client join configuration; encryption is intentionally controlled by the server.
#[derive(Clone)]
pub struct GameClientConfig {
    pub transport: Transport,
    pub bind_addr: Option<SocketAddr>,
    pub remote_addr: SocketAddr,
    /// Opaque application ticket delivered only after the cryptographic handshake.
    pub join_ticket: Vec<u8>,
    pub protocol: GameProtocol,
}

impl GameClientConfig {
    /// Creates a numeric-address client join using the profile's recommended transport.
    pub fn for_profile(
        profile: GameProfile,
        remote_addr: SocketAddr,
        join_ticket: Vec<u8>,
        protocol: GameProtocol,
    ) -> Self {
        Self {
            transport: profile.transport(),
            bind_addr: None,
            remote_addr,
            join_ticket,
            protocol,
        }
    }
}

/// Explicit wire-v4 numeric-address client join.
#[derive(Clone, Debug)]
pub struct GameRangeClientConfig {
    pub transport: Transport,
    pub bind_addr: Option<SocketAddr>,
    pub remote_addr: SocketAddr,
    pub join_ticket: Vec<u8>,
    pub protocol: GameProtocolRange,
}

impl GameRangeClientConfig {
    /// Creates a numeric-address wire-v4 join using the profile's recommended transport.
    pub fn for_profile(
        profile: GameProfile,
        remote_addr: SocketAddr,
        join_ticket: Vec<u8>,
        protocol: GameProtocolRange,
    ) -> Self {
        Self {
            transport: profile.transport(),
            bind_addr: None,
            remote_addr,
            join_ticket,
            protocol,
        }
    }
}

impl std::fmt::Debug for GameClientConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GameClientConfig")
            .field("transport", &self.transport)
            .field("bind_addr", &self.bind_addr)
            .field("remote_addr", &self.remote_addr)
            .field("join_ticket_len", &self.join_ticket.len())
            .field("protocol", &self.protocol)
            .finish()
    }
}

/// Domain-name client join; resolution and cross-address-family retries use the transport layer.
#[derive(Clone)]
pub struct GameHostClientConfig {
    pub transport: Transport,
    pub host: String,
    pub port: u16,
    pub join_ticket: Vec<u8>,
    pub protocol: GameProtocol,
}

impl GameHostClientConfig {
    /// Creates a hostname join using the profile's recommended transport.
    pub fn for_profile(
        profile: GameProfile,
        host: String,
        port: u16,
        join_ticket: Vec<u8>,
        protocol: GameProtocol,
    ) -> Self {
        Self {
            transport: profile.transport(),
            host,
            port,
            join_ticket,
            protocol,
        }
    }
}

/// Explicit wire-v4 hostname client join.
#[derive(Clone, Debug)]
pub struct GameRangeHostClientConfig {
    pub transport: Transport,
    pub host: String,
    pub port: u16,
    pub join_ticket: Vec<u8>,
    pub protocol: GameProtocolRange,
}

impl GameRangeHostClientConfig {
    /// Creates a hostname wire-v4 join using the profile's recommended transport.
    pub fn for_profile(
        profile: GameProfile,
        host: String,
        port: u16,
        join_ticket: Vec<u8>,
        protocol: GameProtocolRange,
    ) -> Self {
        Self {
            transport: profile.transport(),
            host,
            port,
            join_ticket,
            protocol,
        }
    }
}

impl std::fmt::Debug for GameHostClientConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GameHostClientConfig")
            .field("transport", &self.transport)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("join_ticket_len", &self.join_ticket.len())
            .field("protocol", &self.protocol)
            .finish()
    }
}
