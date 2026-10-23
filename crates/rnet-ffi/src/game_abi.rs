//! Current game-facing ABI layouts.

use crate::abi::RNET_ABI_VERSION;
pub use crate::game_metrics_abi::RnetGameMetrics;
use rnet_game::{
    ClockSyncSample, LatestTransportSnapshot, NetworkQuality, QualityBasis, QualityGrade,
};
use std::mem::size_of;

/// Lowest local scheduler priority; no effect on remote dispatch.
pub const RNET_GAME_PRIORITY_LOW: u32 = 1;
/// Default priority used by ordinary game send.
pub const RNET_GAME_PRIORITY_NORMAL: u32 = 2;
/// Higher local scheduler share, not a guaranteed deadline.
pub const RNET_GAME_PRIORITY_HIGH: u32 = 3;
/// Highest local scheduler share; lower queues still receive service.
pub const RNET_GAME_PRIORITY_CRITICAL: u32 = 4;

/// Game runtime started; no peer readiness implied.
pub const RNET_GAME_RUNTIME_STARTED: u32 = 1;
/// Endpoint socket opened; continue polling for authorization/readiness.
pub const RNET_GAME_ENDPOINT_OPENED: u32 = 2;
/// Endpoint-related failure; query validity rather than assuming terminal closure.
pub const RNET_GAME_ENDPOINT_ERROR: u32 = 3;
/// Server authorization: data=login ticket, client key/build/capabilities identify request.
pub const RNET_GAME_AUTH_REQUEST: u32 = 4;
/// Server reauthorization: data=identity, aux_data=fresh login, related_session=old handle.
pub const RNET_GAME_RESUME_REQUEST: u32 = 5;
/// Business protocol/range mismatch before application authorization.
pub const RNET_GAME_PROTOCOL_REJECTED: u32 = 6;
/// Initial game join completed; session can send business data.
pub const RNET_GAME_SESSION_READY: u32 = 7;
/// Resume committed; session=new handle, related_session=old invalidated handle.
pub const RNET_GAME_SESSION_RESUMED: u32 = 8;
/// Client receives sensitive single-runtime, single-use resume credential in data.
pub const RNET_GAME_RESUME_TICKET: u32 = 9;
/// Session ended; status holds local close category.
pub const RNET_GAME_SESSION_CLOSED: u32 = 10;
/// Opaque business data plus optional sequence/tick/correlation metadata.
pub const RNET_GAME_MESSAGE: u32 = 11;
/// Advisory send-capacity recovery; not a reservation or delivery receipt.
pub const RNET_GAME_WRITABLE: u32 = 12;
/// Join/negotiation failed; status describes local category.
pub const RNET_GAME_JOIN_FAILED: u32 = 13;
/// Local mode/key barrier completed; inspect encrypted/epoch/operation.
pub const RNET_GAME_SECURITY_CHANGED: u32 = 14;
/// Quality classification changed; inspect grade/basis/RTT/jitter/samples.
pub const RNET_GAME_QUALITY_CHANGED: u32 = 15;
/// Game/control protocol rejected; never a business message.
pub const RNET_GAME_PROTOCOL_VIOLATION: u32 = 16;
/// Game runtime stop completed; release buffers then destroy.
pub const RNET_GAME_RUNTIME_STOPPED: u32 = 17;

/// Cumulative telemetry for keyed snapshots after game staging.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// TCP/KCP pending-slot statistics; worker pickup ends replacement eligibility.
pub struct RnetGameTransportLatest {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// Cumulative TCP/KCP pending values replaced before I/O worker pickup.
    pub pending_replaced: u64,
    /// Cumulative slots consumed by I/O workers; no recall is possible after pickup.
    pub worker_pickups: u64,
    /// Cumulative bounded transport-capacity rejections, including retries.
    pub admission_would_block: u64,
    /// Cumulative admissions referencing retired/unknown sessions.
    pub admission_invalid_handle: u64,
    /// Cumulative admissions when runtime/session was not accepting work.
    pub admission_invalid_state: u64,
    /// Cumulative admissions before transport readiness.
    pub admission_handshake_required: u64,
    /// Cumulative unsupported-path rejections; UDP uses ordinary forwarding.
    pub admission_not_supported: u64,
    /// Cumulative size-limit rejections.
    pub admission_message_too_large: u64,
    /// Cumulative admission failures outside the explicit categories.
    pub admission_other_failures: u64,
}

impl Default for RnetGameTransportLatest {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            pending_replaced: 0,
            worker_pickups: 0,
            admission_would_block: 0,
            admission_invalid_handle: 0,
            admission_invalid_state: 0,
            admission_handshake_required: 0,
            admission_not_supported: 0,
            admission_message_too_large: 0,
            admission_other_failures: 0,
        }
    }
}

impl From<LatestTransportSnapshot> for RnetGameTransportLatest {
    fn from(value: LatestTransportSnapshot) -> Self {
        Self {
            pending_replaced: value.pending_replaced,
            worker_pickups: value.worker_pickups,
            admission_would_block: value.admission_would_block,
            admission_invalid_handle: value.admission_invalid_handle,
            admission_invalid_state: value.admission_invalid_state,
            admission_handshake_required: value.admission_handshake_required,
            admission_not_supported: value.admission_not_supported,
            admission_message_too_large: value.admission_message_too_large,
            admission_other_failures: value.admission_other_failures,
            ..Self::default()
        }
    }
}

/// Borrowed event data stays valid until the corresponding buffer token is released.
/// `data` is the message, join ticket, resume ticket, or resume identity; `aux_data`
/// holds the resume request's separate join ticket. Callers must release both tokens.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Game event; fields are meaningful only for the indicated event kind. Both tokens are owned.
pub struct RnetGameEvent {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// RNET_GAME_* event discriminant; only the documented fields for that event are meaningful.
    pub event_type: u32,
    /// Local endpoint; zero for runtime-wide events.
    pub endpoint: u64,
    /// Current/pending local session, including NEW handle on SESSION_RESUMED; zero when not applicable.
    pub session: u64,
    /// Old local handle on RESUME_REQUEST/SESSION_RESUMED; zero otherwise.
    pub related_session: u64,
    /// RNET_E_* on error/rejection/close events; other kinds (including PROTOCOL_VIOLATION) use RNET_OK.
    pub status: i32,
    /// AUTH_REQUEST login ticket; RESUME_REQUEST identity; RESUME_TICKET credential; MESSAGE business bytes.
    pub data: *const u8,
    /// data byte count, no NUL terminator; empty data may have NULL pointer.
    pub data_len: usize,
    /// Token owning data; release each nonzero token once before destroy.
    pub buffer_token: u64,
    /// RESUME_REQUEST's fresh login ticket, separate from the identity in data; otherwise empty.
    pub aux_data: *const u8,
    /// aux_data byte count; zero when absent.
    pub aux_data_len: usize,
    /// Independent ownership token for aux_data; release separately from buffer_token.
    pub aux_buffer_token: u64,
    /// Authenticated 32-byte client key on AUTH_REQUEST/RESUME_REQUEST.
    pub client_public_key: [u8; 32],
    /// Client build metadata on AUTH_REQUEST/RESUME_REQUEST.
    pub build_id: u64,
    /// Client capability metadata on AUTH_REQUEST/RESUME_REQUEST; not authorization.
    pub capabilities: u64,
    /// SECURITY_CHANGED resulting business mode: 0 plaintext, 1 encrypted; controls stay protected.
    pub encrypted: u32,
    /// SECURITY_CHANGED session-local key/mode generation, not a wall-clock time.
    pub security_epoch: u64,
    /// SECURITY_CHANGED RNET_SECURITY_OPERATION_* value.
    pub security_operation: u32,
    /// QUALITY_CHANGED grade: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor.
    pub quality_grade: u32,
    /// QUALITY_CHANGED basis: 1 latency, 2 latency+UDP gaps, 3 latency+KCP retransmission.
    pub quality_basis: u32,
    /// QUALITY_CHANGED latest authenticated heartbeat RTT in microseconds.
    pub last_rtt_us: u64,
    /// QUALITY_CHANGED smoothed absolute RTT variation in microseconds.
    pub jitter_us: u64,
    /// QUALITY_CHANGED accepted heartbeat sample count.
    pub quality_samples: u64,
    /// MESSAGE: 0 absent, 1 sender supplied optional sequence metadata.
    pub has_sequence: u32,
    /// MESSAGE sequence if has_sequence=1; not internal UDP loss numbering.
    pub sequence: u32,
    /// MESSAGE: 0 absent, 1 sender supplied a simulation tick.
    pub has_tick: u32,
    /// MESSAGE simulation tick if has_tick=1; not a trusted clock.
    pub tick: u32,
    /// MESSAGE optional request/trace metadata; zero means absent.
    pub correlation_id: u64,
}

impl Default for RnetGameEvent {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            event_type: 0,
            endpoint: 0,
            session: 0,
            related_session: 0,
            status: 0,
            data: std::ptr::null(),
            data_len: 0,
            buffer_token: 0,
            aux_data: std::ptr::null(),
            aux_data_len: 0,
            aux_buffer_token: 0,
            client_public_key: [0; 32],
            build_id: 0,
            capabilities: 0,
            encrypted: 0,
            security_epoch: 0,
            security_operation: 0,
            quality_grade: 0,
            quality_basis: 0,
            last_rtt_us: 0,
            jitter_us: 0,
            quality_samples: 0,
            has_sequence: 0,
            sequence: 0,
            has_tick: 0,
            tick: 0,
            correlation_id: 0,
        }
    }
}

/// Advanced game send metadata. Message type and transport routing remain library-owned.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Game metadata/scheduling. Priority must be explicit; zero is not Normal.
pub struct RnetGameSendOptions {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// 0 omits sequence, 1 includes it; no ordering or deduplication is enabled.
    pub has_sequence: u32,
    /// Caller-defined sequence copied unchanged when has_sequence=1; zero is valid.
    pub sequence: u32,
    /// 0 omits tick, 1 includes it.
    pub has_tick: u32,
    /// Caller-defined simulation tick copied unchanged when has_tick=1.
    pub tick: u32,
    /// Optional request/trace identifier copied to the peer; zero absent.
    pub correlation_id: u64,
    /// Required RNET_GAME_PRIORITY_*; zero is invalid. Only local scheduling is affected.
    pub priority: u32,
    /// Relative local queue lifetime. Zero means no expiry.
    /// Relative local staging lifetime in milliseconds; zero means no expiry, not immediate expiry.
    pub expiry_ms: u64,
}

impl Default for RnetGameSendOptions {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            has_sequence: 0,
            sequence: 0,
            has_tick: 0,
            tick: 0,
            correlation_id: 0,
            priority: RNET_GAME_PRIORITY_NORMAL,
            expiry_ms: 0,
        }
    }
}

/// A sample is unavailable until the first authenticated heartbeat acknowledgement.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Heartbeat quality with transport-specific loss evidence, not universal packet loss.
pub struct RnetGameQuality {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// 0 before the first heartbeat sample; 1 permits interpreting the remaining measurements.
    pub available: u32,
    /// 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor; sample minima may keep grade unknown.
    pub grade: u32,
    /// 1 latency only, 2 latency+UDP gaps, 3 latency+KCP retransmissions.
    pub basis: u32,
    /// 0 unavailable, 1 UDP loss fields present; not the same as measured zero loss.
    pub has_udp_loss: u32,
    /// 0 unavailable, 1 KCP fields present; TCP has neither transport signal.
    pub has_kcp_retransmissions: u32,
    /// Recent UDP sequence-gap estimate per 1,000 positions (10 means 1%); check availability.
    pub udp_recent_loss_per_mille: u32,
    /// Recent KCP retransmission ratio per 1,000 PUSH transmissions; not raw network loss.
    pub kcp_recent_retransmission_per_mille: u32,
    /// Latest authenticated heartbeat RTT in microseconds, including queueing effects.
    pub last_rtt_us: u64,
    /// Smoothed heartbeat RTT in microseconds; not a percentile.
    pub smoothed_rtt_us: u64,
    /// Smoothed absolute RTT variation in microseconds.
    pub jitter_us: u64,
    /// Accepted heartbeat samples on this session.
    pub samples: u64,
    /// Cumulative expected UDP data positions, excluding implausible forward jumps.
    pub udp_expected: u64,
    /// Expected minus received UDP positions; late arrivals may reduce this value.
    pub udp_missing: u64,
    /// Cumulative KCP PUSH transmissions, including retransmissions, excluding ACKs.
    pub kcp_segments_sent: u64,
    /// Cumulative repeated KCP PUSH transmissions.
    pub kcp_retransmitted: u64,
}

impl Default for RnetGameQuality {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            available: 0,
            grade: 0,
            basis: 0,
            has_udp_loss: 0,
            has_kcp_retransmissions: 0,
            udp_recent_loss_per_mille: 0,
            kcp_recent_retransmission_per_mille: 0,
            last_rtt_us: 0,
            smoothed_rtt_us: 0,
            jitter_us: 0,
            samples: 0,
            udp_expected: 0,
            udp_missing: 0,
            kcp_segments_sent: 0,
            kcp_retransmitted: 0,
        }
    }
}

impl From<NetworkQuality> for RnetGameQuality {
    fn from(value: NetworkQuality) -> Self {
        let mut out = Self {
            available: 1,
            grade: quality_grade(value.grade),
            basis: quality_basis(value.basis),
            last_rtt_us: micros(value.last_rtt),
            smoothed_rtt_us: micros(value.smoothed_rtt),
            jitter_us: micros(value.jitter),
            samples: value.samples,
            ..Self::default()
        };
        if let Some(loss) = value.udp_loss {
            out.has_udp_loss = 1;
            out.udp_recent_loss_per_mille = u32::from(loss.recent_loss_per_mille);
            out.udp_expected = loss.expected;
            out.udp_missing = loss.missing;
        }
        if let Some(retransmissions) = value.kcp_retransmissions {
            out.has_kcp_retransmissions = 1;
            out.kcp_recent_retransmission_per_mille =
                u32::from(retransmissions.recent_retransmission_per_mille);
            out.kcp_segments_sent = retransmissions.segments_sent;
            out.kcp_retransmitted = retransmissions.retransmitted;
        }
        out
    }
}

/// `server_minus_client_us` maps this client runtime's monotonic clock to the
/// server runtime's monotonic clock; it is not a UTC or security time source.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Approximate client-side monotonic clock sample; not UTC or an anti-cheat time authority.
pub struct RnetGameClockSync {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// 0 on server sessions or before first sample; 1 when a client sample is available.
    pub available: u32,
    /// Reserved output, written as zero by the library.
    pub reserved: u32,
    /// Signed microseconds added to client runtime clock to approximate server runtime clock.
    pub server_minus_client_us: i64,
    /// Round-trip estimate in microseconds after subtracting reported server turnaround.
    pub rtt_us: u64,
    /// Accepted clock samples on this client session, not heartbeat samples.
    pub samples: u64,
}

impl Default for RnetGameClockSync {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            available: 0,
            reserved: 0,
            server_minus_client_us: 0,
            rtt_us: 0,
            samples: 0,
        }
    }
}

impl From<ClockSyncSample> for RnetGameClockSync {
    fn from(sample: ClockSyncSample) -> Self {
        Self {
            available: 1,
            server_minus_client_us: sample.server_minus_client_us,
            rtt_us: micros(sample.rtt),
            samples: sample.samples,
            ..Self::default()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Owned export buffer; caller releases its token instead of freeing data.
pub struct RnetGameBuffer {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// Library-owned bytes, valid until token release or runtime destruction; not NUL-terminated.
    pub data: *const u8,
    /// Number of bytes in data.
    pub len: usize,
    /// Release each nonzero token once with rnet_game_buffer_release on the owning runtime.
    pub token: u64,
}

impl Default for RnetGameBuffer {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            data: std::ptr::null(),
            len: 0,
            token: 0,
        }
    }
}

pub(crate) fn quality_grade(value: QualityGrade) -> u32 {
    match value {
        QualityGrade::Unknown => 0,
        QualityGrade::Excellent => 1,
        QualityGrade::Good => 2,
        QualityGrade::Fair => 3,
        QualityGrade::Poor => 4,
    }
}

pub(crate) fn quality_basis(value: QualityBasis) -> u32 {
    match value {
        QualityBasis::LatencyOnly => 1,
        QualityBasis::UdpSequenceGap => 2,
        QualityBasis::KcpRetransmission => 3,
    }
}

pub(crate) fn micros(value: std::time::Duration) -> u64 {
    value.as_micros().min(u128::from(u64::MAX)) as u64
}
