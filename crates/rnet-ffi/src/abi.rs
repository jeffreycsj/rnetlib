use rnet_core::ErrorCode;
use rnet_core::EventType;
use rnet_core::Transport;
use rnet_observe::LogLevel;
use rnet_transport::LatencyKind;
use rnet_transport::RuntimeConfig;
use std::ffi::c_void;
use std::mem::size_of;

pub const RNET_ABI_VERSION: u32 = 1;
pub const RNET_OK: i32 = ErrorCode::Ok as i32;
pub const RNET_E_INVALID_ARGUMENT: i32 = ErrorCode::InvalidArgument as i32;
pub const RNET_E_INVALID_HANDLE: i32 = ErrorCode::InvalidHandle as i32;
pub const RNET_E_INVALID_STATE: i32 = ErrorCode::InvalidState as i32;
pub const RNET_E_WOULD_BLOCK: i32 = ErrorCode::WouldBlock as i32;
pub const RNET_E_TIMEOUT: i32 = ErrorCode::Timeout as i32;
pub const RNET_E_NOT_SUPPORTED: i32 = ErrorCode::NotSupported as i32;
pub const RNET_E_IO_ERROR: i32 = ErrorCode::IoError as i32;
pub const RNET_E_PROTOCOL_ERROR: i32 = ErrorCode::ProtocolError as i32;
pub const RNET_E_MESSAGE_TOO_LARGE: i32 = ErrorCode::MessageTooLarge as i32;
pub const RNET_E_INTERNAL_PANIC: i32 = ErrorCode::InternalPanic as i32;
pub const RNET_E_HANDSHAKE_REQUIRED: i32 = ErrorCode::HandshakeRequired as i32;
pub const RNET_E_AUTH_REJECTED: i32 = ErrorCode::AuthRejected as i32;
pub const RNET_E_PEER_KEY_MISMATCH: i32 = ErrorCode::PeerKeyMismatch as i32;
pub const RNET_E_CANCELLED: i32 = ErrorCode::Cancelled as i32;

pub const RNET_LOG_TRACE: u32 = LogLevel::Trace as u32;
pub const RNET_LOG_DEBUG: u32 = LogLevel::Debug as u32;
pub const RNET_LOG_INFO: u32 = LogLevel::Info as u32;
pub const RNET_LOG_WARN: u32 = LogLevel::Warn as u32;
pub const RNET_LOG_ERROR: u32 = LogLevel::Error as u32;

pub const RNET_TRANSPORT_TCP: u32 = Transport::Tcp as u32;
pub const RNET_TRANSPORT_UDP: u32 = Transport::Udp as u32;
pub const RNET_TRANSPORT_KCP: u32 = Transport::Kcp as u32;
pub const RNET_SECURITY_PLAINTEXT: u32 = 1;
pub const RNET_SECURITY_ENCRYPTED: u32 = 2;

pub const RNET_LATENCY_CONNECT: u32 = LatencyKind::Connect as u32;
pub const RNET_LATENCY_CRYPTO_HANDSHAKE: u32 = LatencyKind::CryptoHandshake as u32;
pub const RNET_LATENCY_AUTH_WAIT: u32 = LatencyKind::AuthWait as u32;
pub const RNET_LATENCY_SEND_QUEUE: u32 = LatencyKind::SendQueue as u32;
pub const RNET_LATENCY_EVENT_QUEUE: u32 = LatencyKind::EventQueue as u32;
pub const RNET_LATENCY_KCP_RTT: u32 = LatencyKind::KcpRtt as u32;
pub const RNET_LATENCY_KCP_UPDATE_DELAY: u32 = LatencyKind::KcpUpdateDelay as u32;
pub const RNET_LATENCY_LOGGER_CALLBACK: u32 = LatencyKind::LoggerCallback as u32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl Default for RnetSlice {
    fn default() -> Self {
        Self {
            ptr: std::ptr::null(),
            len: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetLogger {
    pub struct_size: u32,
    pub abi_version: u32,
    pub log: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            level: u32,
            target: *const u8,
            target_len: usize,
            message: *const u8,
            message_len: usize,
        ),
    >,
    pub user_data: *mut c_void,
    pub min_level: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetLoggerV2 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub log: Option<RnetLogV2Fn>,
    pub user_data: *mut c_void,
    pub min_level: u32,
}

pub type RnetLogV2Fn = unsafe extern "C" fn(
    user_data: *mut c_void,
    timestamp_unix_ms: u64,
    level: u32,
    event_name: *const u8,
    event_name_len: usize,
    runtime: u64,
    endpoint: u64,
    session: u64,
    transport: u32,
    error_code: i32,
    correlation_id: u64,
    message: *const u8,
    message_len: usize,
);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub worker_threads: u32,
    pub event_queue_capacity: u32,
    pub write_queue_capacity: u32,
    pub max_body_len: u32,
    pub max_datagram_size: u32,
    pub logger: *const RnetLogger,
}

/// Production runtime configuration. New SDKs use this structure; the legacy layout is unchanged.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetConfigV3 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub worker_threads: u32,
    pub event_queue_capacity: u32,
    pub write_queue_capacity: u32,
    pub max_body_len: u32,
    pub max_datagram_size: u32,
    pub max_event_bytes: u64,
    pub max_runtime_queued_bytes: u64,
    pub max_session_queued_bytes: u64,
    pub max_sessions_per_endpoint: u32,
    pub max_sessions_per_ip: u32,
    pub handshake_rate_per_ip: u32,
    pub handshake_burst_per_ip: u32,
    pub ipv6_admission_prefix_bits: u32,
    pub handshake_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub dns_timeout_ms: u64,
    pub datagram_idle_timeout_ms: u64,
    pub allow_plaintext_business_data: u32,
    pub allow_legacy_unauthenticated_endpoints: u32,
    pub rekey_after_ms: u64,
    pub rekey_after_bytes: u64,
    pub logger: *const RnetLogger,
    pub logger_v2: *const RnetLoggerV2,
}

impl Default for RnetConfigV3 {
    fn default() -> Self {
        let defaults = RuntimeConfig::production();
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            worker_threads: defaults.worker_threads.min(u32::MAX as usize) as u32,
            event_queue_capacity: defaults.event_queue_capacity as u32,
            write_queue_capacity: defaults.write_queue_capacity as u32,
            max_body_len: defaults.max_body_len as u32,
            max_datagram_size: defaults.max_datagram_size as u32,
            max_event_bytes: defaults.max_event_bytes as u64,
            max_runtime_queued_bytes: defaults.max_runtime_queued_bytes as u64,
            max_session_queued_bytes: defaults.max_session_queued_bytes as u64,
            max_sessions_per_endpoint: defaults.max_sessions_per_endpoint as u32,
            max_sessions_per_ip: defaults.max_sessions_per_ip as u32,
            handshake_rate_per_ip: defaults.handshake_rate_per_ip,
            handshake_burst_per_ip: defaults.handshake_burst_per_ip,
            ipv6_admission_prefix_bits: u32::from(defaults.ipv6_admission_prefix_bits),
            handshake_timeout_ms: defaults.handshake_timeout.as_millis() as u64,
            connect_timeout_ms: defaults.connect_timeout.as_millis() as u64,
            dns_timeout_ms: defaults.dns_timeout.as_millis() as u64,
            datagram_idle_timeout_ms: defaults.datagram_idle_timeout.as_millis() as u64,
            allow_plaintext_business_data: 0,
            allow_legacy_unauthenticated_endpoints: 0,
            rekey_after_ms: defaults
                .security_policy
                .rekey_after
                .map_or(0, |duration| duration.as_millis() as u64),
            rekey_after_bytes: defaults.security_policy.rekey_after_bytes.unwrap_or(0),
            logger: std::ptr::null(),
            logger_v2: std::ptr::null(),
        }
    }
}

impl Default for RnetConfig {
    fn default() -> Self {
        let defaults = RuntimeConfig::default();
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            worker_threads: defaults.worker_threads.min(u32::MAX as usize) as u32,
            event_queue_capacity: defaults.event_queue_capacity as u32,
            write_queue_capacity: defaults.write_queue_capacity as u32,
            max_body_len: defaults.max_body_len as u32,
            max_datagram_size: defaults.max_datagram_size as u32,
            logger: std::ptr::null(),
        }
    }
}

/// Runtime-scoped client identity and server trust policy.
///
/// The callback may run on an I/O worker thread and must remain callable until runtime destroy.
/// When `verify_server` is absent, `expected_server_public_key` must contain one 32-byte key.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetClientSecurity {
    pub struct_size: u32,
    pub abi_version: u32,
    pub local_private_key: RnetSlice,
    pub expected_server_public_key: RnetSlice,
    pub verify_server: Option<unsafe extern "C" fn(*mut c_void, *const u8, usize) -> u32>,
    pub user_data: *mut c_void,
}

impl RnetClientSecurity {
    pub fn pinned(local_private_key: RnetSlice, expected_server_public_key: RnetSlice) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            local_private_key,
            expected_server_public_key,
            verify_server: None,
            user_data: std::ptr::null_mut(),
        }
    }
}

/// Transport-neutral server listener configuration.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetServerConfigV2 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub initial_security: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved: u16,
    pub local_private_key: RnetSlice,
}

impl RnetServerConfigV2 {
    pub fn new(
        transport: u32,
        bind_host: RnetSlice,
        bind_port: u16,
        local_private_key: RnetSlice,
        initial_security: u32,
    ) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            transport,
            initial_security,
            bind_host,
            bind_port,
            reserved: 0,
            local_private_key,
        }
    }
}

/// Transport-neutral client configuration; security is inherited from the runtime and server.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetClientConfigV2 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved0: u32,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved1: u16,
    pub join_payload: RnetSlice,
}

impl RnetClientConfigV2 {
    pub fn new(
        transport: u32,
        remote_host: RnetSlice,
        remote_port: u16,
        join_payload: RnetSlice,
    ) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            transport,
            reserved0: 0,
            remote_host,
            remote_port,
            reserved1: 0,
            join_payload,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RnetEndpointMode {
    Listener = 1,
    Client = 2,
    Datagram = 3,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetEndpointConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub mode: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved0: u16,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved1: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetKeypair {
    pub struct_size: u32,
    pub abi_version: u32,
    pub private_key: [u8; 32],
    pub public_key: [u8; 32],
}

impl Default for RnetKeypair {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            private_key: [0; 32],
            public_key: [0; 32],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetListenerConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved0: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved1: u16,
    pub local_private_key: RnetSlice,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetJoinConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved0: u32,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved1: u16,
    pub local_private_key: RnetSlice,
    pub expected_server_public_key: RnetSlice,
    pub join_payload: RnetSlice,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetSendOptions {
    pub struct_size: u32,
    pub abi_version: u32,
    pub correlation_id: u64,
    pub flags: u32,
    pub reserved: u32,
}

impl Default for RnetSendOptions {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            correlation_id: 0,
            flags: 0,
            reserved: 0,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RnetEventType {
    RuntimeStarted = EventType::RuntimeStarted as u32,
    EndpointOpened = EventType::EndpointOpened as u32,
    EndpointError = EventType::EndpointError as u32,
    SessionOpened = EventType::SessionOpened as u32,
    SessionClosed = EventType::SessionClosed as u32,
    Message = EventType::Message as u32,
    Writable = EventType::Writable as u32,
    RuntimeStopped = EventType::RuntimeStopped as u32,
    AuthRequest = EventType::AuthRequest as u32,
    JoinFailed = EventType::JoinFailed as u32,
    SecurityChanged = EventType::SecurityChanged as u32,
    GameControl = EventType::GameControl as u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetEvent {
    pub struct_size: u32,
    pub event_type: u32,
    pub endpoint: u64,
    pub session: u64,
    pub msg_type: u32,
    pub stream_id: u32,
    pub request_id: u64,
    pub data: *const u8,
    pub data_len: usize,
    pub buffer_token: u64,
    pub status: i32,
}

impl Default for RnetEvent {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            event_type: 0,
            endpoint: 0,
            session: 0,
            msg_type: 0,
            stream_id: 0,
            request_id: 0,
            data: std::ptr::null(),
            data_len: 0,
            buffer_token: 0,
            status: RNET_OK,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RnetMetrics {
    pub struct_size: u32,
    pub abi_version: u32,
    pub frames_received: u64,
    pub frames_sent: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub events_dropped: u64,
    pub send_would_block: u64,
    pub protocol_errors: u64,
    pub logs_dropped: u64,
    pub logger_panics: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RnetLatencyMetric {
    pub struct_size: u32,
    pub kind: u32,
    pub sample_count: u64,
    pub p50_us: u64,
    pub p90_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RnetMetricsV2 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub frames_received: u64,
    pub frames_sent: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub events_dropped: u64,
    pub send_would_block: u64,
    pub protocol_errors: u64,
    pub lifecycle_events_rejected: u64,
    pub admission_rejected: u64,
    pub queued_send_bytes: u64,
    pub peak_queued_send_bytes: u64,
    pub queued_event_bytes: u64,
    pub session_closed_by_reason: [u64; 19],
    pub logs_dropped: u64,
    pub logger_panics: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RnetMetricsV3 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub frames_received: u64,
    pub frames_sent: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub events_dropped: u64,
    pub send_would_block: u64,
    pub protocol_errors: u64,
    pub lifecycle_events_rejected: u64,
    pub admission_rejected: u64,
    pub queued_send_bytes: u64,
    pub peak_queued_send_bytes: u64,
    pub queued_event_bytes: u64,
    pub session_closed_by_reason: [u64; 19],
    pub logs_dropped: u64,
    pub logger_panics: u64,
    pub current_endpoints: u64,
    pub current_sessions: u64,
    pub established_sessions: u64,
    pub pending_handshakes: u64,
    pub peak_pending_handshakes: u64,
    pub admission_rejected_by_reason: [u64; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RnetLatencyMetricV2 {
    pub struct_size: u32,
    pub kind: u32,
    pub sample_count: u64,
    pub p50_us: u64,
    pub p90_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
}
