use rnet_core::ErrorCode;
use rnet_core::Result;
use rnet_core::RnetError;
use rnet_core::Transport;
use rnet_security::Keypair;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Runtime-wide policy for capabilities that weaken transport security.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecurityPolicy {
    /// Allows business records without confidentiality or integrity after authenticated setup.
    pub allow_plaintext_business_data: bool,
    /// Allows the compatibility TCP/UDP endpoints that do not perform authentication.
    pub allow_legacy_unauthenticated_endpoints: bool,
    /// Optional time threshold for a server-led automatic rekey.
    pub rekey_after: Option<Duration>,
    /// Optional encrypted application-byte threshold for a server-led automatic rekey.
    pub rekey_after_bytes: Option<u64>,
}

impl SecurityPolicy {
    pub const fn production() -> Self {
        Self {
            allow_plaintext_business_data: false,
            allow_legacy_unauthenticated_endpoints: false,
            rekey_after: Some(Duration::from_secs(60 * 60)),
            rekey_after_bytes: Some(1024 * 1024 * 1024),
        }
    }

    pub const fn compatibility() -> Self {
        Self {
            allow_plaintext_business_data: true,
            allow_legacy_unauthenticated_endpoints: true,
            rekey_after: None,
            rekey_after_bytes: None,
        }
    }
}

/// Thread-safe callback used by clients to authenticate a server's static Noise public key.
pub type PeerVerifier = Arc<dyn Fn(&[u8; 32]) -> bool + Send + Sync + 'static>;

/// Runtime-wide client identity and server trust policy used by unified connections.
#[derive(Clone)]
pub struct ClientSecurity {
    pub(crate) local_key: Keypair,
    pub(crate) peer_verifier: PeerVerifier,
}

impl std::fmt::Debug for ClientSecurity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientSecurity")
            .field("local_public_key", &self.local_key.public)
            .field("peer_verifier", &"<callback>")
            .finish()
    }
}

impl ClientSecurity {
    /// Creates a client policy that accepts only one exact 32-byte server public key.
    pub fn pinned(local_key: Keypair, expected_server_public: Vec<u8>) -> Self {
        let verifier =
            Arc::new(move |actual: &[u8; 32]| expected_server_public.as_slice() == actual);
        Self {
            local_key,
            peer_verifier: verifier,
        }
    }

    /// Creates a client policy backed by an application trust callback.
    pub fn with_verifier(local_key: Keypair, peer_verifier: PeerVerifier) -> Self {
        Self {
            local_key,
            peer_verifier,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub worker_threads: usize,
    pub event_queue_capacity: usize,
    pub write_queue_capacity: usize,
    pub max_body_len: usize,
    pub max_datagram_size: usize,
    /// Maximum bytes retained by the application event queue.
    pub max_event_bytes: usize,
    /// Maximum encoded application bytes waiting to be written across the runtime.
    pub max_runtime_queued_bytes: usize,
    /// Maximum encoded application bytes waiting to be written for one session.
    pub max_session_queued_bytes: usize,
    pub security_policy: SecurityPolicy,
    /// Maximum number of concurrently open listeners and client endpoints in one runtime.
    pub max_endpoints: usize,
    /// Maximum cryptographic or application-authentication handshakes across the runtime.
    pub max_pending_handshakes: usize,
    /// Maximum pending plus established sessions owned by one listener endpoint.
    pub max_sessions_per_endpoint: usize,
    /// Maximum concurrent TCP handshakes/sessions admitted from one IP or IPv6 prefix.
    pub max_sessions_per_ip: usize,
    /// Sustained new TCP handshakes admitted per second from one IP or IPv6 prefix.
    pub handshake_rate_per_ip: u32,
    /// Maximum instantaneous handshake burst from one IP or IPv6 prefix.
    pub handshake_burst_per_ip: u32,
    /// Prefix width used to aggregate IPv6 admission accounting.
    pub ipv6_admission_prefix_bits: u8,
    /// Deadline for transport-independent cryptographic and application authentication setup.
    pub handshake_timeout: Duration,
    /// Per-address TCP connection deadline.
    pub connect_timeout: Duration,
    /// Total deadline for one host-name resolution operation.
    pub dns_timeout: Duration,
    /// Disables Nagle's algorithm for latency-sensitive TCP messages.
    pub tcp_nodelay: bool,
    /// Requested kernel TCP send-buffer size; `None` preserves the operating-system default.
    pub tcp_send_buffer_bytes: Option<usize>,
    /// Requested kernel TCP receive-buffer size; `None` preserves the operating-system default.
    pub tcp_recv_buffer_bytes: Option<usize>,
    /// Maximum receive inactivity for established UDP/KCP peers.
    pub datagram_idle_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            worker_threads: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
            event_queue_capacity: 4096,
            write_queue_capacity: 256,
            max_body_len: 1024 * 1024,
            max_datagram_size: 1200,
            max_event_bytes: 64 * 1024 * 1024,
            max_runtime_queued_bytes: 256 * 1024 * 1024,
            max_session_queued_bytes: 4 * 1024 * 1024,
            security_policy: SecurityPolicy::compatibility(),
            max_endpoints: 1024,
            max_pending_handshakes: 16_384,
            max_sessions_per_endpoint: 16_384,
            max_sessions_per_ip: 256,
            handshake_rate_per_ip: 100,
            handshake_burst_per_ip: 200,
            ipv6_admission_prefix_bits: 64,
            handshake_timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(5),
            dns_timeout: Duration::from_secs(5),
            tcp_nodelay: true,
            tcp_send_buffer_bytes: None,
            tcp_recv_buffer_bytes: None,
            datagram_idle_timeout: Duration::from_secs(120),
        }
    }
}

impl RuntimeConfig {
    /// Returns secure defaults for new deployments while `Default` preserves legacy behavior.
    pub fn production() -> Self {
        Self {
            security_policy: SecurityPolicy::production(),
            ..Self::default()
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let now = Instant::now();
        if self.worker_threads == 0
            || self.event_queue_capacity == 0
            || self.write_queue_capacity == 0
            || self.max_body_len == 0
            || self.max_body_len > u32::MAX as usize
            || self.max_datagram_size < rnet_protocol::HEADER_LEN
            || self.max_datagram_size > 65_507
            || self.max_event_bytes == 0
            || self.max_runtime_queued_bytes == 0
            || self.max_session_queued_bytes == 0
            || self.max_session_queued_bytes > self.max_runtime_queued_bytes
            || self.max_endpoints == 0
            || self.max_pending_handshakes == 0
            || self.max_sessions_per_endpoint == 0
            || self.max_sessions_per_ip == 0
            || self.handshake_rate_per_ip == 0
            || self.handshake_burst_per_ip == 0
            || self.ipv6_admission_prefix_bits > 128
            || self.handshake_timeout.is_zero()
            || now.checked_add(self.handshake_timeout).is_none()
            || self.connect_timeout.is_zero()
            || now.checked_add(self.connect_timeout).is_none()
            || self.dns_timeout.is_zero()
            || now.checked_add(self.dns_timeout).is_none()
            || self.tcp_send_buffer_bytes == Some(0)
            || self.tcp_recv_buffer_bytes == Some(0)
            || self.datagram_idle_timeout.is_zero()
            || now.checked_add(self.datagram_idle_timeout).is_none()
            || self
                .security_policy
                .rekey_after
                .is_some_and(|duration| duration.is_zero() || now.checked_add(duration).is_none())
            || self.security_policy.rekey_after_bytes == Some(0)
        {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid runtime capacity or size limit",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct EndpointConfig {
    pub transport: Transport,
    pub bind_addr: Option<SocketAddr>,
    pub remote_addr: Option<SocketAddr>,
    pub listen: bool,
    pub security: Option<EndpointSecurity>,
}

/// Configuration for a server listener.
///
/// The transport is selected once when the listener is created. Application sends only refer to
/// a session, so callers cannot accidentally change a live session from TCP to UDP or KCP.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// TCP, UDP, or KCP; fixed for every session accepted by this listener.
    pub transport: Transport,
    /// Numeric local address on which the server receives connections.
    pub bind_addr: SocketAddr,
    /// Server Noise identity copied into the endpoint task.
    pub local_key: Keypair,
    /// Initial protection of business records; authenticated control is always encrypted.
    pub initial_security: rnet_protocol::control::SecurityMode,
}

/// Configuration for a client connection.
///
/// Security is deliberately absent: the client follows the server's authenticated policy. The
/// runtime-level [`ClientSecurity`] supplies client identity and server trust verification.
#[derive(Clone)]
pub struct ClientConfig {
    /// TCP, UDP, or KCP; must match the server listener.
    pub transport: Transport,
    /// Optional numeric local address. UDP and KCP normally use a wildcard address with port zero.
    pub bind_addr: Option<SocketAddr>,
    /// Numeric server address.
    pub remote_addr: SocketAddr,
    /// Opaque bytes delivered to the server's authentication event inside the Noise handshake.
    pub join_payload: Vec<u8>,
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientConfig")
            .field("transport", &self.transport)
            .field("bind_addr", &self.bind_addr)
            .field("remote_addr", &self.remote_addr)
            .field("join_payload_len", &self.join_payload.len())
            .finish()
    }
}

/// Client configuration for a DNS host name or textual IP address.
#[derive(Clone)]
pub struct HostClientConfig {
    pub transport: Transport,
    pub host: String,
    pub port: u16,
    pub join_payload: Vec<u8>,
}

impl std::fmt::Debug for HostClientConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostClientConfig")
            .field("transport", &self.transport)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("join_payload_len", &self.join_payload.len())
            .finish()
    }
}

/// Client configuration with caller-resolved candidates in retry order.
#[derive(Clone)]
pub struct ResolvedClientConfig {
    pub transport: Transport,
    pub remote_addrs: Vec<SocketAddr>,
    pub join_payload: Vec<u8>,
}

impl std::fmt::Debug for ResolvedClientConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedClientConfig")
            .field("transport", &self.transport)
            .field("remote_addrs", &self.remote_addrs)
            .field("join_payload_len", &self.join_payload.len())
            .finish()
    }
}

impl From<ServerConfig> for EndpointConfig {
    fn from(config: ServerConfig) -> Self {
        Self::server(
            config.transport,
            config.bind_addr,
            config.local_key,
            config.initial_security,
        )
    }
}

impl From<ClientConfig> for EndpointConfig {
    fn from(config: ClientConfig) -> Self {
        Self::client(
            config.transport,
            config.bind_addr,
            config.remote_addr,
            config.join_payload,
        )
    }
}

#[derive(Clone)]
pub enum EndpointSecurity {
    Server {
        local_key: Keypair,
    },
    Client {
        local_key: Keypair,
        expected_server_public: Vec<u8>,
        join_payload: Vec<u8>,
    },
    AdaptiveServer {
        local_key: Keypair,
        initial_mode: rnet_protocol::control::SecurityMode,
    },
    AdaptiveClient {
        join_payload: Vec<u8>,
    },
}

impl std::fmt::Debug for EndpointSecurity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Server { local_key } => formatter
                .debug_struct("Server")
                .field("local_key", local_key)
                .finish(),
            Self::Client {
                local_key,
                expected_server_public,
                join_payload,
            } => formatter
                .debug_struct("Client")
                .field("local_key", local_key)
                .field("expected_server_public", expected_server_public)
                .field("join_payload_len", &join_payload.len())
                .finish(),
            Self::AdaptiveServer {
                local_key,
                initial_mode,
            } => formatter
                .debug_struct("AdaptiveServer")
                .field("local_key", local_key)
                .field("initial_mode", initial_mode)
                .finish(),
            Self::AdaptiveClient { join_payload } => formatter
                .debug_struct("AdaptiveClient")
                .field("join_payload_len", &join_payload.len())
                .finish(),
        }
    }
}

impl EndpointConfig {
    /// Creates a unified server listener whose initial data protection is server controlled.
    pub fn server(
        transport: Transport,
        bind_addr: SocketAddr,
        local_key: Keypair,
        initial_mode: rnet_protocol::control::SecurityMode,
    ) -> Self {
        Self {
            transport,
            bind_addr: Some(bind_addr),
            remote_addr: None,
            listen: true,
            security: Some(EndpointSecurity::AdaptiveServer {
                local_key,
                initial_mode,
            }),
        }
    }

    /// Creates a unified client connection. Security is negotiated and verified automatically.
    pub fn client(
        transport: Transport,
        bind_addr: Option<SocketAddr>,
        remote_addr: SocketAddr,
        join_payload: Vec<u8>,
    ) -> Self {
        Self {
            transport,
            bind_addr,
            remote_addr: Some(remote_addr),
            listen: false,
            security: Some(EndpointSecurity::AdaptiveClient { join_payload }),
        }
    }

    pub fn tcp_listener(bind_addr: SocketAddr) -> Self {
        Self {
            transport: Transport::Tcp,
            bind_addr: Some(bind_addr),
            remote_addr: None,
            listen: true,
            security: None,
        }
    }

    pub fn tcp_client(remote_addr: SocketAddr) -> Self {
        Self {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: Some(remote_addr),
            listen: false,
            security: None,
        }
    }

    pub fn udp(bind_addr: SocketAddr, remote_addr: Option<SocketAddr>) -> Self {
        Self {
            transport: Transport::Udp,
            bind_addr: Some(bind_addr),
            remote_addr,
            listen: false,
            security: None,
        }
    }

    pub fn kcp(bind_addr: SocketAddr) -> Self {
        Self {
            transport: Transport::Kcp,
            bind_addr: Some(bind_addr),
            remote_addr: None,
            listen: false,
            security: None,
        }
    }

    pub fn secure_tcp_listener(bind_addr: SocketAddr, local_key: Keypair) -> Self {
        Self {
            security: Some(EndpointSecurity::Server { local_key }),
            ..Self::tcp_listener(bind_addr)
        }
    }

    pub fn secure_tcp_client(
        remote_addr: SocketAddr,
        local_key: Keypair,
        expected_server_public: Vec<u8>,
        join_payload: Vec<u8>,
    ) -> Self {
        Self {
            security: Some(EndpointSecurity::Client {
                local_key,
                expected_server_public,
                join_payload,
            }),
            ..Self::tcp_client(remote_addr)
        }
    }

    pub fn secure_udp_listener(bind_addr: SocketAddr, local_key: Keypair) -> Self {
        Self {
            transport: Transport::Udp,
            bind_addr: Some(bind_addr),
            remote_addr: None,
            listen: true,
            security: Some(EndpointSecurity::Server { local_key }),
        }
    }

    pub fn secure_udp_client(
        bind_addr: SocketAddr,
        remote_addr: SocketAddr,
        local_key: Keypair,
        expected_server_public: Vec<u8>,
        join_payload: Vec<u8>,
    ) -> Self {
        Self {
            transport: Transport::Udp,
            bind_addr: Some(bind_addr),
            remote_addr: Some(remote_addr),
            listen: false,
            security: Some(EndpointSecurity::Client {
                local_key,
                expected_server_public,
                join_payload,
            }),
        }
    }

    pub fn secure_kcp_listener(bind_addr: SocketAddr, local_key: Keypair) -> Self {
        let mut config = Self::secure_udp_listener(bind_addr, local_key);
        config.transport = Transport::Kcp;
        config
    }

    pub fn secure_kcp_client(
        bind_addr: SocketAddr,
        remote_addr: SocketAddr,
        local_key: Keypair,
        expected_server_public: Vec<u8>,
        join_payload: Vec<u8>,
    ) -> Self {
        let mut config = Self::secure_udp_client(
            bind_addr,
            remote_addr,
            local_key,
            expected_server_public,
            join_payload,
        );
        config.transport = Transport::Kcp;
        config
    }
}
