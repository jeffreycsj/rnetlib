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

/// Protocol identity and inclusive version interval for the current authenticated join.
///
/// Build and capability values remain authenticated application metadata. They do not grant
/// library capabilities. A range is valid only when its ID and both bounds are nonzero and the
/// lower bound does not exceed the upper bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameProtocol {
    /// Nonzero application protocol identity; both peers must use the same value.
    pub protocol_id: u64,
    /// Lowest supported application version, inclusive and nonzero.
    pub min_version: u32,
    /// Highest supported version, inclusive and at least `min_version`.
    /// A new join selects the highest version supported by both peers.
    pub max_version: u32,
    /// Client build metadata exposed during server authorization; zero means unspecified.
    /// Not used to select a version, and ignored in listener configuration.
    pub build_id: u64,
    /// Client-defined capability bits exposed during authorization; zero advertises none.
    /// The library neither interprets these bits nor negotiates features from them.
    pub capabilities: u64,
}

impl GameProtocol {
    /// Describes supported versions; endpoint creation validates the ID and bounds.
    /// Use equal bounds to require one exact application version.
    /// `protocol_id` names the application protocol, not the TCP/UDP/KCP transport or SDK wire.
    /// Build metadata and capability bits start at zero; this constructor does not validate.
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
/// Plaintext business traffic is an explicit policy choice, not an SDK compatibility mode.
///
/// ```compile_fail,E0599
/// let _ = rnet_game::GameRuntimeConfig::compatibility();
/// ```
#[derive(Clone, Debug)]
pub struct GameRuntimeConfig {
    /// Low-level capacity, timeout, socket, and security policy.
    pub network: RuntimeConfig,
    /// Positive interval between authenticated probes after an acknowledgement; default 5 s.
    /// Probes and deadlines advance only while the caller continues polling.
    pub heartbeat_interval: Duration,
    /// Positive acknowledgement deadline; must be at least `heartbeat_interval`, default 15 s.
    pub heartbeat_timeout: Duration,
    /// Thresholds for RTT/jitter and, when available, UDP sequence-gap classification.
    pub quality_policy: QualityPolicy,
    /// Independent bounded staging budget for coalesced realtime state messages.
    pub realtime_queue: RealtimeQueueConfig,
    /// Bounded staging for advanced priority/expiry sends.
    pub scheduled_queue: ScheduledQueueConfig,
    /// Lifetime of a one-use, client-key-bound credential: greater than zero and at most 300 s.
    /// Default 30 s. Tickets cannot survive destruction of the issuing runtime.
    pub resume_ticket_ttl: Duration,
    /// Cap on outstanding credentials in this runtime: 1..=65,536; default 65,536.
    /// State is neither persisted nor shared across processes.
    pub max_resume_tickets: usize,
}

impl GameRuntimeConfig {
    /// Secure defaults for a new deployment.
    /// This is a configuration preset, not a claim of capacity or production qualification.
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
    /// `interval` and `timeout` must be positive and `timeout >= interval`.
    /// Validation occurs when creating the runtime, not in this builder.
    pub fn with_heartbeat(mut self, interval: Duration, timeout: Duration) -> Self {
        self.heartbeat_interval = interval;
        self.heartbeat_timeout = timeout;
        self
    }

    /// Replaces quality thresholds with `policy`; runtime creation validates their ordering.
    pub fn with_quality_policy(mut self, policy: QualityPolicy) -> Self {
        self.quality_policy = policy;
        self
    }

    /// Replaces snapshot staging limits with `queue`; zero limits are rejected at creation.
    pub fn with_realtime_queue(mut self, queue: RealtimeQueueConfig) -> Self {
        self.realtime_queue = queue;
        self
    }

    /// Replaces ordinary/priority send staging limits; does not change transport queue limits.
    pub fn with_scheduled_queue(mut self, queue: ScheduledQueueConfig) -> Self {
        self.scheduled_queue = queue;
        self
    }

    /// Sets ticket lifetime `ttl` in `(0, 300 s]` and capacity `max_tickets` in `1..=65_536`.
    /// Runtime creation validates these values; zero does not mean unlimited or disabled.
    pub fn with_resume(mut self, ttl: Duration, max_tickets: usize) -> Self {
        self.resume_ticket_ttl = ttl;
        self.max_resume_tickets = max_tickets;
        self
    }

    /// Explicitly permits a local server to switch business records to plaintext.
    ///
    /// This cannot bypass connection authentication. Noise setup and security-control
    /// messages stay authenticated; only subsequent business payloads lose confidentiality and
    /// integrity while the server-selected plaintext mode is active.
    /// Clients need no matching opt-in: they follow their authenticated server's mode.
    pub fn allow_plaintext_business_data(mut self, allow: bool) -> Self {
        self.network.security_policy.allow_plaintext_business_data = allow;
        self
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
    /// One transport for this listener and all accepted sessions; cannot change in place.
    pub transport: Transport,
    /// Local numeric interface and port; port zero requests an OS-assigned port.
    pub bind_addr: SocketAddr,
    /// Server Noise identity; clients must trust its public key through their security config.
    pub local_key: Keypair,
    /// Initial business-data protection. The server may change it after authorization.
    /// `false` requires the runtime's explicit plaintext policy opt-in.
    pub initial_encryption: bool,
    /// Inclusive application version range required before application authorization.
    pub protocol: GameProtocol,
}

impl GameServerConfig {
    /// Creates an encrypted listener using the profile's recommended transport.
    /// `profile` selects only transport defaults; `bind_addr`, `local_key` and `protocol`
    /// have the same meanings as their fields. Validation and binding occur in `listen`.
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

/// Client join configuration; encryption is intentionally controlled by the server.
#[derive(Clone)]
pub struct GameClientConfig {
    /// Must match the remote listener; fixed for the lifetime of this endpoint.
    pub transport: Transport,
    /// Optional local bind address for datagram transports. `None` chooses a matching
    /// wildcard interface and ephemeral port. TCP uses its transport-managed local binding.
    pub bind_addr: Option<SocketAddr>,
    /// Numeric server address and port; server trust is configured on the runtime, not here.
    pub remote_addr: SocketAddr,
    /// Opaque application ticket delivered only after the cryptographic handshake.
    /// May be empty if server policy permits. Join framing plus credentials must fit the
    /// runtime body limit and the 60 KiB handshake ceiling. Never log these bytes.
    pub join_ticket: Vec<u8>,
    /// Supported application versions and client metadata for server authorization.
    pub protocol: GameProtocol,
}

impl GameClientConfig {
    /// Creates a numeric-address client join using the profile's recommended transport.
    /// `remote_addr`, `join_ticket` and `protocol` populate the corresponding fields;
    /// local binding is automatic. This only builds configuration, not a connection.
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
    /// Must match the remote listener; independent of how its address is resolved.
    pub transport: Transport,
    /// DNS hostname or numeric IP literal, without URL scheme, path or embedded port.
    pub host: String,
    /// Remote service port; must be nonzero for hostname connection APIs.
    pub port: u16,
    /// Opaque login credential, with the same sensitivity and size rules as numeric joins.
    pub join_ticket: Vec<u8>,
    /// Supported application versions and authenticated client build/capability metadata.
    pub protocol: GameProtocol,
}

impl GameHostClientConfig {
    /// Creates a hostname join using the profile's recommended transport.
    /// `host` and `port` remain separate; `join_ticket` and `protocol` populate their fields.
    /// Resolution and validation happen on connect, not during configuration construction.
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
