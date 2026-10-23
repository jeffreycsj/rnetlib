use rnet_core::ErrorCode;
use rnet_core::EventType;
use rnet_core::Transport;
use rnet_observe::LogLevel;
use rnet_transport::LatencyKind;
use std::ffi::c_void;
use std::mem::size_of;

/// Exact SDK/native ABI layout identifier, not the application protocol version.
pub const RNET_ABI_VERSION: u32 = 4;
/// Success; no local library error.
pub const RNET_OK: i32 = 0;
const _: () = assert!(RNET_OK == ErrorCode::Ok as i32);
/// Invalid value, layout, pointer presence or unsupported field combination.
pub const RNET_E_INVALID_ARGUMENT: i32 = -1;
const _: () = assert!(RNET_E_INVALID_ARGUMENT == ErrorCode::InvalidArgument as i32);
/// Unknown, stale, foreign or already released handle/token.
pub const RNET_E_INVALID_HANDLE: i32 = -2;
const _: () = assert!(RNET_E_INVALID_HANDLE == ErrorCode::InvalidHandle as i32);
/// Operation is not allowed in the current lifecycle/security state.
pub const RNET_E_INVALID_STATE: i32 = -3;
const _: () = assert!(RNET_E_INVALID_STATE == ErrorCode::InvalidState as i32);
/// Bounded admission or lifecycle progress blocked; poll/drain and retry appropriately.
pub const RNET_E_WOULD_BLOCK: i32 = -4;
const _: () = assert!(RNET_E_WOULD_BLOCK == ErrorCode::WouldBlock as i32);
/// Local operation/peer deadline expired.
pub const RNET_E_TIMEOUT: i32 = -5;
const _: () = assert!(RNET_E_TIMEOUT == ErrorCode::Timeout as i32);
/// Operation not supported by the selected transport/role.
pub const RNET_E_NOT_SUPPORTED: i32 = -6;
const _: () = assert!(RNET_E_NOT_SUPPORTED == ErrorCode::NotSupported as i32);
/// Local socket/runtime I/O failure; inspect diagnostic and affected handle.
pub const RNET_E_IO_ERROR: i32 = -7;
const _: () = assert!(RNET_E_IO_ERROR == ErrorCode::IoError as i32);
/// Invalid network protocol/frame or incompatible peer behavior.
pub const RNET_E_PROTOCOL_ERROR: i32 = -8;
const _: () = assert!(RNET_E_PROTOCOL_ERROR == ErrorCode::ProtocolError as i32);
/// Payload exceeds the applicable frame/datagram/budget limit.
pub const RNET_E_MESSAGE_TOO_LARGE: i32 = -9;
const _: () = assert!(RNET_E_MESSAGE_TOO_LARGE == ErrorCode::MessageTooLarge as i32);
/// Caught internal Rust panic; not a peer's business error.
pub const RNET_E_INTERNAL_PANIC: i32 = -10;
const _: () = assert!(RNET_E_INTERNAL_PANIC == ErrorCode::InternalPanic as i32);
/// Session is not yet ready for the requested operation.
pub const RNET_E_HANDSHAKE_REQUIRED: i32 = -11;
const _: () = assert!(RNET_E_HANDSHAKE_REQUIRED == ErrorCode::HandshakeRequired as i32);
/// Server application authorization denied.
pub const RNET_E_AUTH_REJECTED: i32 = -13;
const _: () = assert!(RNET_E_AUTH_REJECTED == ErrorCode::AuthRejected as i32);
/// Presented server key was not trusted by the client.
pub const RNET_E_PEER_KEY_MISMATCH: i32 = -14;
const _: () = assert!(RNET_E_PEER_KEY_MISMATCH == ErrorCode::PeerKeyMismatch as i32);
/// Operation/session explicitly cancelled.
pub const RNET_E_CANCELLED: i32 = -18;
const _: () = assert!(RNET_E_CANCELLED == ErrorCode::Cancelled as i32);

/// Most verbose diagnostics.
pub const RNET_LOG_TRACE: u32 = 0;
const _: () = assert!(RNET_LOG_TRACE == LogLevel::Trace as u32);
/// Development/debug diagnostics.
pub const RNET_LOG_DEBUG: u32 = 1;
const _: () = assert!(RNET_LOG_DEBUG == LogLevel::Debug as u32);
/// Normal lifecycle/summary information.
pub const RNET_LOG_INFO: u32 = 2;
const _: () = assert!(RNET_LOG_INFO == LogLevel::Info as u32);
/// Recoverable issue or anomalous peer behavior.
pub const RNET_LOG_WARN: u32 = 3;
const _: () = assert!(RNET_LOG_WARN == LogLevel::Warn as u32);
/// Operation/endpoint/runtime failure.
pub const RNET_LOG_ERROR: u32 = 4;
const _: () = assert!(RNET_LOG_ERROR == LogLevel::Error as u32);

/// Reliable ordered stream transport with library message framing.
pub const RNET_TRANSPORT_TCP: u32 = 1;
const _: () = assert!(RNET_TRANSPORT_TCP == Transport::Tcp as u32);
/// Unreliable message transport; no retransmission or delivery guarantee.
pub const RNET_TRANSPORT_UDP: u32 = 2;
const _: () = assert!(RNET_TRANSPORT_UDP == Transport::Udp as u32);
/// Reliable ordered KCP delivery over UDP.
pub const RNET_TRANSPORT_KCP: u32 = 3;
const _: () = assert!(RNET_TRANSPORT_KCP == Transport::Kcp as u32);
/// Unprotected business bytes; protected controls and peer handshake remain enabled.
pub const RNET_SECURITY_PLAINTEXT: u32 = 1;
/// Authenticated encrypted business bytes and protected controls.
pub const RNET_SECURITY_ENCRYPTED: u32 = 2;

/// TCP connect operation duration.
pub const RNET_LATENCY_CONNECT: u32 = 1;
const _: () = assert!(RNET_LATENCY_CONNECT == LatencyKind::Connect as u32);
/// Cryptographic handshake duration.
pub const RNET_LATENCY_CRYPTO_HANDSHAKE: u32 = 2;
const _: () = assert!(RNET_LATENCY_CRYPTO_HANDSHAKE == LatencyKind::CryptoHandshake as u32);
/// Server application authorization wait duration.
pub const RNET_LATENCY_AUTH_WAIT: u32 = 3;
const _: () = assert!(RNET_LATENCY_AUTH_WAIT == LatencyKind::AuthWait as u32);
/// Time waiting for transport send processing.
pub const RNET_LATENCY_SEND_QUEUE: u32 = 4;
const _: () = assert!(RNET_LATENCY_SEND_QUEUE == LatencyKind::SendQueue as u32);
/// Time waiting for application event polling.
pub const RNET_LATENCY_EVENT_QUEUE: u32 = 5;
const _: () = assert!(RNET_LATENCY_EVENT_QUEUE == LatencyKind::EventQueue as u32);
/// KCP acknowledgement-based RTT samples, not game heartbeat RTT.
pub const RNET_LATENCY_KCP_RTT: u32 = 6;
const _: () = assert!(RNET_LATENCY_KCP_RTT == LatencyKind::KcpRtt as u32);
/// Observed KCP update scheduling delay.
pub const RNET_LATENCY_KCP_UPDATE_DELAY: u32 = 7;
const _: () = assert!(RNET_LATENCY_KCP_UPDATE_DELAY == LatencyKind::KcpUpdateDelay as u32);
/// Time spent executing the installed log callback.
pub const RNET_LATENCY_LOGGER_CALLBACK: u32 = 8;
const _: () = assert!(RNET_LATENCY_LOGGER_CALLBACK == LatencyKind::LoggerCallback as u32);

#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Length-delimited borrowed bytes; strings use UTF-8 without a required terminator.
pub struct RnetSlice {
    /// Borrowed bytes; may be NULL only when len is zero.
    pub ptr: *const u8,
    /// Byte count, not a NUL-terminated string length including its terminator.
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
/// Optional bounded asynchronous log sink. Descriptor is copied; context is not owned.
pub struct RnetLogger {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// Required callback; called serially on a dedicated logging thread.
    // cbindgen needs the nullable signature inline; the equality check below
    // preserves the existing RnetLogFn alias without changing this ABI.
    #[allow(clippy::type_complexity)]
    pub log: Option<
        unsafe extern "C" fn(
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
        ),
    >,
    /// Opaque callback context; must remain valid until runtime destruction returns.
    pub user_data: *mut c_void,
    /// Minimum RNET_LOG_* severity, inclusive; values above ERROR clamp to ERROR.
    pub min_level: u32,
}

/// Logging callback: user_data is the registered context; timestamp_unix_ms is UTC
/// milliseconds, level is RNET_LOG_*. event_name/message are borrowed length-delimited
/// UTF-8 valid only during this call, not necessarily NUL-terminated. Handles,
/// transport and correlation_id describe the source; zero means unspecified.
/// error_code is RNET_OK or the local RNET_E_* category.
/// Runs serially on this runtime's logging thread; keep it short to avoid bounded
/// queue drops. Must not throw/unwind across C, or call stop/destroy from the callback.
pub type RnetLogFn = unsafe extern "C" fn(
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

/// Runtime-scoped client identity and server trust policy.
///
/// The callback may run on an I/O worker thread and must remain callable until runtime destroy.
/// When `verify_server` is absent, `expected_server_public_key` must contain one 32-byte key.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Client identity and server trust for all outbound connections from a runtime.
pub struct RnetClientSecurity {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// Exactly 32 private-key bytes, copied during creation; caller erases its own copy.
    pub local_private_key: RnetSlice,
    /// Exactly 32 pinned public-key bytes when verify_server is NULL; otherwise ignored.
    pub expected_server_public_key: RnetSlice,
    /// Optional server-key trust callback; overrides the exact public-key pin.
    pub verify_server: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            public_key: *const u8,
            public_key_len: usize,
        ) -> u32,
    >,
    /// Opaque verifier context; must outlive runtime destruction and in-flight verification.
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
/// Authenticated listener configuration; transport is fixed for its lifetime.
pub struct RnetServerConfig {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// One RNET_TRANSPORT_* value, fixed for this listener and its sessions.
    pub transport: u32,
    /// RNET_SECURITY_* business-data mode; plaintext requires runtime opt-in.
    pub initial_security: u32,
    /// UTF-8 numeric IPv4/IPv6 without brackets, URL scheme or port; DNS is not accepted.
    pub bind_host: RnetSlice,
    /// Local port; zero asks the OS to choose an ephemeral port.
    pub bind_port: u16,
    /// Reserved; must be zero.
    pub reserved: u16,
    /// Exactly 32 server identity bytes, copied by listen; never log them.
    pub local_private_key: RnetSlice,
}

impl RnetServerConfig {
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
/// Outbound endpoint configuration; identity/trust belong to the runtime.
pub struct RnetClientConfig {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// RNET_TRANSPORT_* matching the listener; fixed for the connection.
    pub transport: u32,
    /// Reserved; must be zero.
    pub reserved0: u32,
    /// UTF-8 DNS name or numeric IP, without URL scheme, path or embedded port.
    pub remote_host: RnetSlice,
    /// Remote service port; must be nonzero.
    pub remote_port: u16,
    /// Reserved; must be zero.
    pub reserved1: u16,
    /// Opaque credential/application join bytes inside the protected handshake; copied on connect.
    pub join_payload: RnetSlice,
}

impl RnetClientConfig {
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

#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Generated identity; contains secret material that the caller must erase.
pub struct RnetKeypair {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// Secret 32-byte key; caller must erase it when no longer needed.
    pub private_key: [u8; 32],
    /// Corresponding 32-byte public key for authenticated peer trust configuration.
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
/// Optional transport trace metadata; zero-initialize then set the exact ABI header.
pub struct RnetSendOptions {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// Optional request/trace metadata copied to the peer; zero means absent.
    pub correlation_id: u64,
    /// Must be zero; no user-selectable framing flags exist in this ABI.
    pub flags: u32,
    /// Reserved; must be zero.
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
/// Transport event. Release every nonzero token, including diagnostic/auth payloads.
/// SESSION_CLOSED, JOIN_FAILED and ENDPOINT_ERROR may carry local UTF-8 diagnostics.
/// Diagnostic bytes may be absent under event-byte pressure; status remains authoritative.
/// SECURITY_CHANGED data is mode:u8 | epoch:u64 big-endian | operation:u8, emitted
/// after the local barrier commits. AUTH_REQUEST data is client_public_key[32]
/// followed by the opaque join payload.
pub struct RnetEvent {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// One RNET_EVENT_* discriminant; determines the meaning of data and status.
    pub event_type: u32,
    /// Local owning endpoint; zero for runtime-wide events.
    pub endpoint: u64,
    /// Local session; zero for runtime/endpoint events or failures before allocation.
    pub session: u64,
    /// MESSAGE request/trace metadata; zero when absent.
    pub correlation_id: u64,
    /// Owned-by-library payload or diagnostic bytes; valid until buffer_token is released or runtime destroyed.
    pub data: *const u8,
    /// Number of data bytes; no trailing NUL is implied. Zero permits a NULL data pointer.
    pub data_len: usize,
    /// Release each nonzero token exactly once using this runtime's rnet_buffer_release.
    pub buffer_token: u64,
    /// Local RNET_OK or negative RNET_E_* category; not a peer's business error code.
    pub status: i32,
}

impl Default for RnetEvent {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            event_type: 0,
            endpoint: 0,
            session: 0,
            correlation_id: 0,
            data: std::ptr::null(),
            data_len: 0,
            buffer_token: 0,
            status: RNET_OK,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
/// Runtime-wide counters/gauges; concurrent fields are not a globally atomic snapshot.
pub struct RnetMetrics {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// Cumulative logical frames received, not raw IP packets.
    pub frames_received: u64,
    /// Cumulative logical frames accepted for transport sending, not remote acknowledgements.
    pub frames_sent: u64,
    /// Cumulative decoded frame body bytes received; excludes outer framing.
    pub bytes_received: u64,
    /// Cumulative logical encoded frame bytes sent; includes logical framing.
    pub bytes_sent: u64,
    /// Cumulative transport event queue admission failures.
    pub events_dropped: u64,
    /// Cumulative transport send backpressure rejections.
    pub send_would_block: u64,
    /// Cumulative malformed/invalid transport-protocol input observations.
    pub protocol_errors: u64,
    /// Cumulative failed lifecycle-event admissions; cleanup does not depend on delivery.
    pub lifecycle_events_rejected: u64,
    /// Cumulative endpoint/session/handshake admission failures.
    pub admission_rejected: u64,
    /// Current send byte reservations; includes framing, not total process RSS.
    pub queued_send_bytes: u64,
    /// Highest runtime send byte reservation observed.
    pub peak_queued_send_bytes: u64,
    /// Current payload bytes awaiting transport poll; excludes exported buffer tokens.
    pub queued_event_bytes: u64,
    /// Cumulative closes indexed by negated error code, 0..18 (RNET_OK maps to 0).
    pub session_closed_by_reason: [u64; 19],
    /// Cumulative bounded logger admission drops; zero also when no logger is installed.
    pub logs_dropped: u64,
    /// Cumulative caught Rust logger panics; foreign exceptions must never cross the C boundary.
    pub logger_panics: u64,
    /// Current registered endpoints, including listeners and client connections.
    pub current_endpoints: u64,
    /// Current registered sessions including unfinished handshakes.
    pub current_sessions: u64,
    /// Current transport-open sessions; not proof of game readiness.
    pub established_sessions: u64,
    /// Current sessions awaiting transport handshake/authorization.
    pub pending_handshakes: u64,
    /// Highest pending-handshake count observed.
    pub peak_pending_handshakes: u64,
    /// Counts: endpoint, endpoint-session, pending, IP-active, IP-rate, IP-table, KCP-peer, unauthenticated.
    pub admission_rejected_by_reason: [u64; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
/// Approximate histogram percentiles; microseconds, with sample_count indicating availability.
pub struct RnetLatencyMetric {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// RNET_LATENCY_* operation sampled; units below are microseconds.
    pub kind: u32,
    /// Samples represented; zero means unavailable, not measured zero latency.
    pub sample_count: u64,
    /// Estimated median latency; histogram bucket approximation.
    pub p50_us: u64,
    /// Estimated 90th percentile latency.
    pub p90_us: u64,
    /// Estimated 95th percentile latency.
    pub p95_us: u64,
    /// Estimated 99th percentile latency.
    pub p99_us: u64,
    /// Estimated 99.9th percentile latency.
    pub p999_us: u64,
    /// Largest sampled duration; concurrent snapshots are not atomic across all fields.
    pub max_us: u64,
}

/// Trust decision for a server public_key (public_key_len is 32), borrowed only
/// during the call; user_data is the registered context. Nonzero accepts, zero
/// rejects. May run concurrently on I/O workers: context must be thread-safe.
/// Callback/context must outlive runtime destruction. Do not block, throw/unwind
/// across C, or stop/destroy this runtime from an I/O callback.
pub type RnetPeerVerifyFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    public_key: *const u8,
    public_key_len: usize,
) -> u32;

/// Cryptographic or protected join exchange failed.
pub const RNET_E_HANDSHAKE_FAILED: i32 = -12;
const _: () = assert!(RNET_E_HANDSHAKE_FAILED == ErrorCode::HandshakeFailed as i32);

/// Protected sequence/replay validation failed.
pub const RNET_E_REPLAY_DETECTED: i32 = -15;
const _: () = assert!(RNET_E_REPLAY_DETECTED == ErrorCode::ReplayDetected as i32);

/// Encryption/decryption/authentication operation failed.
pub const RNET_E_CRYPTO_ERROR: i32 = -16;
const _: () = assert!(RNET_E_CRYPTO_ERROR == ErrorCode::CryptoError as i32);

/// Peer admission or request rate limit exceeded.
pub const RNET_E_RATE_LIMITED: i32 = -17;
const _: () = assert!(RNET_E_RATE_LIMITED == ErrorCode::RateLimited as i32);

/// Business encryption policy barrier completed locally.
pub const RNET_SECURITY_OPERATION_MODE_SWITCH: u32 = 1;

/// Key-generation barrier completed locally.
pub const RNET_SECURITY_OPERATION_REKEY: u32 = 2;

/// Runtime started; endpoint/session zero.
pub const RNET_EVENT_RUNTIME_STARTED: u32 = 1;
const _: () = assert!(RNET_EVENT_RUNTIME_STARTED == EventType::RuntimeStarted as u32);

/// Endpoint socket became available; not a ready session.
pub const RNET_EVENT_ENDPOINT_OPENED: u32 = 2;
const _: () = assert!(RNET_EVENT_ENDPOINT_OPENED == EventType::EndpointOpened as u32);

/// Endpoint-related failure; may be recoverable, query endpoint before assuming closure.
pub const RNET_EVENT_ENDPOINT_ERROR: u32 = 3;
const _: () = assert!(RNET_EVENT_ENDPOINT_ERROR == EventType::EndpointError as u32);

/// Transport handshake and authorization completed; not game readiness.
pub const RNET_EVENT_SESSION_OPENED: u32 = 4;
const _: () = assert!(RNET_EVENT_SESSION_OPENED == EventType::SessionOpened as u32);

/// Peer closed; status is local reason, data may contain a diagnostic.
pub const RNET_EVENT_SESSION_CLOSED: u32 = 5;
const _: () = assert!(RNET_EVENT_SESSION_CLOSED == EventType::SessionClosed as u32);

/// Opaque transport payload with optional correlation metadata.
pub const RNET_EVENT_MESSAGE: u32 = 6;
const _: () = assert!(RNET_EVENT_MESSAGE == EventType::Message as u32);

/// Advisory send-capacity recovery; another send can still encounter backpressure.
pub const RNET_EVENT_WRITABLE: u32 = 7;
const _: () = assert!(RNET_EVENT_WRITABLE == EventType::Writable as u32);

/// Runtime stop completed; release remaining buffers before destroy.
pub const RNET_EVENT_RUNTIME_STOPPED: u32 = 8;
const _: () = assert!(RNET_EVENT_RUNTIME_STOPPED == EventType::RuntimeStopped as u32);

/// Server must decide; data contains key[32] followed by opaque join payload.
pub const RNET_EVENT_AUTH_REQUEST: u32 = 9;
const _: () = assert!(RNET_EVENT_AUTH_REQUEST == EventType::AuthRequest as u32);

/// Outbound connection/handshake failed; status/detail explain the failure.
pub const RNET_EVENT_JOIN_FAILED: u32 = 10;
const _: () = assert!(RNET_EVENT_JOIN_FAILED == EventType::JoinFailed as u32);

/// Local barrier committed; data has mode, big-endian epoch, operation (10 bytes).
pub const RNET_EVENT_SECURITY_CHANGED: u32 = 11;
const _: () = assert!(RNET_EVENT_SECURITY_CHANGED == EventType::SecurityChanged as u32);

/// Internal protected game-control frame; not business data.
pub const RNET_EVENT_GAME_CONTROL: u32 = 12;
const _: () = assert!(RNET_EVENT_GAME_CONTROL == EventType::GameControl as u32);

// cbindgen cannot resolve Option<callback typedef>. Keep the inline nullable
// fields source-compatible with the public bare function aliases at compile time.
const _: fn(RnetLogger) -> Option<RnetLogFn> = |value| value.log;
const _: fn(RnetClientSecurity) -> Option<RnetPeerVerifyFn> = |value| value.verify_server;
