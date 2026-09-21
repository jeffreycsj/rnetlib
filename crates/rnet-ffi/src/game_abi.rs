//! Additive game-facing ABI layouts. The existing transport ABI remains frozen.

use crate::abi::{RnetSlice, RNET_ABI_VERSION};
use crate::abi_config::RnetConfigV5;
use rnet_game::{
    ClockSyncSample, GameRuntime, LatestTransportSnapshot, NetworkQuality, QualityBasis,
    QualityGrade, RealtimeQueueSnapshot,
};
use std::mem::size_of;

pub const RNET_GAME_RUNTIME_STARTED: u32 = 1;
pub const RNET_GAME_ENDPOINT_OPENED: u32 = 2;
pub const RNET_GAME_ENDPOINT_ERROR: u32 = 3;
pub const RNET_GAME_AUTH_REQUEST: u32 = 4;
pub const RNET_GAME_RESUME_REQUEST: u32 = 5;
pub const RNET_GAME_PROTOCOL_REJECTED: u32 = 6;
pub const RNET_GAME_SESSION_READY: u32 = 7;
pub const RNET_GAME_SESSION_RESUMED: u32 = 8;
pub const RNET_GAME_RESUME_TICKET: u32 = 9;
pub const RNET_GAME_SESSION_CLOSED: u32 = 10;
pub const RNET_GAME_MESSAGE: u32 = 11;
pub const RNET_GAME_WRITABLE: u32 = 12;
pub const RNET_GAME_JOIN_FAILED: u32 = 13;
pub const RNET_GAME_SECURITY_CHANGED: u32 = 14;
pub const RNET_GAME_QUALITY_CHANGED: u32 = 15;
pub const RNET_GAME_PROTOCOL_VIOLATION: u32 = 16;
pub const RNET_GAME_RUNTIME_STOPPED: u32 = 17;

/// Zero values on the optional timing fields select the Rust production defaults.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    pub allow_plaintext_business_data: u32,
    pub reserved: u32,
    /// Optional transport limits; borrowed only during runtime creation.
    pub network_config: *const RnetConfigV5,
}

impl Default for RnetGameConfig {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            heartbeat_interval_ms: 0,
            heartbeat_timeout_ms: 0,
            allow_plaintext_business_data: 0,
            reserved: 0,
            network_config: std::ptr::null(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameServerConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub initial_encryption: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved: u16,
    pub local_private_key: RnetSlice,
    pub protocol_id: u64,
    pub protocol_version: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameClientConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved: u32,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved2: u16,
    pub join_ticket: RnetSlice,
    pub protocol_id: u64,
    pub protocol_version: u32,
    pub reserved3: u32,
    pub build_id: u64,
    pub capabilities: u64,
}

/// Explicit wire-v4 listener. Existing exact-version game config retains wire v3.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRangeServerConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub initial_encryption: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved: u16,
    pub local_private_key: RnetSlice,
    pub protocol_id: u64,
    pub min_version: u32,
    pub max_version: u32,
}

/// Explicit wire-v4 hostname join, including metadata authenticated in the join payload.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRangeClientConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved: u32,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved2: u16,
    pub join_ticket: RnetSlice,
    pub protocol_id: u64,
    pub min_version: u32,
    pub max_version: u32,
    pub reserved3: u32,
    pub build_id: u64,
    pub capabilities: u64,
}

/// Cumulative telemetry for keyed snapshots after game staging.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameTransportLatest {
    pub struct_size: u32,
    pub abi_version: u32,
    pub pending_replaced: u64,
    pub worker_pickups: u64,
    pub admission_would_block: u64,
    pub admission_invalid_handle: u64,
    pub admission_invalid_state: u64,
    pub admission_handshake_required: u64,
    pub admission_not_supported: u64,
    pub admission_message_too_large: u64,
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
pub struct RnetGameEvent {
    pub struct_size: u32,
    pub event_type: u32,
    pub endpoint: u64,
    pub session: u64,
    pub related_session: u64,
    pub status: i32,
    pub data: *const u8,
    pub data_len: usize,
    pub buffer_token: u64,
    pub aux_data: *const u8,
    pub aux_data_len: usize,
    pub aux_buffer_token: u64,
    pub client_public_key: [u8; 32],
    pub build_id: u64,
    pub capabilities: u64,
    pub encrypted: u32,
    pub security_epoch: u64,
    pub security_operation: u32,
    pub quality_grade: u32,
    pub quality_basis: u32,
    pub last_rtt_us: u64,
    pub jitter_us: u64,
    pub quality_samples: u64,
    pub has_sequence: u32,
    pub sequence: u32,
    pub has_tick: u32,
    pub tick: u32,
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
        }
    }
}

/// A sample is unavailable until the first authenticated heartbeat acknowledgement.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameQuality {
    pub struct_size: u32,
    pub abi_version: u32,
    pub available: u32,
    pub grade: u32,
    pub basis: u32,
    pub has_udp_loss: u32,
    pub has_kcp_retransmissions: u32,
    pub udp_recent_loss_per_mille: u32,
    pub kcp_recent_retransmission_per_mille: u32,
    pub last_rtt_us: u64,
    pub smoothed_rtt_us: u64,
    pub jitter_us: u64,
    pub samples: u64,
    pub udp_expected: u64,
    pub udp_missing: u64,
    pub kcp_segments_sent: u64,
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
pub struct RnetGameClockSync {
    pub struct_size: u32,
    pub abi_version: u32,
    pub available: u32,
    pub reserved: u32,
    pub server_minus_client_us: i64,
    pub rtt_us: u64,
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

/// Cumulative game-level counters; heartbeat RTT fields are cumulative histogram estimates.
/// The logger fields are meaningful only when `logger_available` is one. There are no
/// per-session labels, player identifiers, credentials, or business payloads in this structure.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameMetrics {
    pub struct_size: u32,
    pub abi_version: u32,
    pub logger_available: u32,
    pub reserved: u32,
    pub heartbeat_probes_sent: u64,
    pub heartbeat_probe_send_failures: u64,
    pub heartbeat_replies_sent: u64,
    pub heartbeat_reply_send_failures: u64,
    pub heartbeat_replies_matched: u64,
    pub heartbeat_replies_rejected: u64,
    pub heartbeat_probes_rate_limited: u64,
    pub heartbeat_timeouts: u64,
    pub heartbeat_rtt_samples: u64,
    pub heartbeat_rtt_p50_us: u64,
    pub heartbeat_rtt_p90_us: u64,
    pub heartbeat_rtt_p95_us: u64,
    pub heartbeat_rtt_p99_us: u64,
    pub heartbeat_rtt_p999_us: u64,
    pub heartbeat_rtt_max_us: u64,
    pub resume_tickets_issued: u64,
    pub resume_requests_received: u64,
    pub resume_tickets_rejected: u64,
    pub resume_authorization_denied: u64,
    pub resume_pending_revoked: u64,
    pub resume_sessions_resumed: u64,
    pub resume_outstanding_tickets: u64,
    pub clock_probes_sent: u64,
    pub clock_replies_sent: u64,
    pub clock_samples: u64,
    pub clock_rejected: u64,
    pub clock_send_failures: u64,
    pub logger_dropped: u64,
    pub logger_sink_panics: u64,
    pub logger_callback_samples: u64,
    pub logger_callback_p99_us: u64,
    pub logger_callback_max_us: u64,
}

impl Default for RnetGameMetrics {
    fn default() -> Self {
        // All metric values start at zero; the fixed ABI header is the only exception.
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            logger_available: 0,
            reserved: 0,
            heartbeat_probes_sent: 0,
            heartbeat_probe_send_failures: 0,
            heartbeat_replies_sent: 0,
            heartbeat_reply_send_failures: 0,
            heartbeat_replies_matched: 0,
            heartbeat_replies_rejected: 0,
            heartbeat_probes_rate_limited: 0,
            heartbeat_timeouts: 0,
            heartbeat_rtt_samples: 0,
            heartbeat_rtt_p50_us: 0,
            heartbeat_rtt_p90_us: 0,
            heartbeat_rtt_p95_us: 0,
            heartbeat_rtt_p99_us: 0,
            heartbeat_rtt_p999_us: 0,
            heartbeat_rtt_max_us: 0,
            resume_tickets_issued: 0,
            resume_requests_received: 0,
            resume_tickets_rejected: 0,
            resume_authorization_denied: 0,
            resume_pending_revoked: 0,
            resume_sessions_resumed: 0,
            resume_outstanding_tickets: 0,
            clock_probes_sent: 0,
            clock_replies_sent: 0,
            clock_samples: 0,
            clock_rejected: 0,
            clock_send_failures: 0,
            logger_dropped: 0,
            logger_sink_panics: 0,
            logger_callback_samples: 0,
            logger_callback_p99_us: 0,
            logger_callback_max_us: 0,
        }
    }
}

impl RnetGameMetrics {
    pub(crate) fn from_runtime(runtime: &GameRuntime) -> Self {
        let heartbeat = runtime.heartbeat_metrics_snapshot();
        let resume = runtime.resume_metrics_snapshot();
        let clock = runtime.clock_sync_metrics_snapshot();
        let mut out = Self {
            heartbeat_probes_sent: heartbeat.probes_sent,
            heartbeat_probe_send_failures: heartbeat.probe_send_failures,
            heartbeat_replies_sent: heartbeat.replies_sent,
            heartbeat_reply_send_failures: heartbeat.reply_send_failures,
            heartbeat_replies_matched: heartbeat.replies_matched,
            heartbeat_replies_rejected: heartbeat.replies_rejected,
            heartbeat_probes_rate_limited: heartbeat.probes_rate_limited,
            heartbeat_timeouts: heartbeat.timeouts,
            heartbeat_rtt_samples: heartbeat.rtt.sample_count,
            heartbeat_rtt_p50_us: heartbeat.rtt.p50_us,
            heartbeat_rtt_p90_us: heartbeat.rtt.p90_us,
            heartbeat_rtt_p95_us: heartbeat.rtt.p95_us,
            heartbeat_rtt_p99_us: heartbeat.rtt.p99_us,
            heartbeat_rtt_p999_us: heartbeat.rtt.p999_us,
            heartbeat_rtt_max_us: heartbeat.rtt.max_us,
            resume_tickets_issued: resume.tickets_issued,
            resume_requests_received: resume.requests_received,
            resume_tickets_rejected: resume.tickets_rejected,
            resume_authorization_denied: resume.authorization_denied,
            resume_pending_revoked: resume.pending_revoked,
            resume_sessions_resumed: resume.sessions_resumed,
            resume_outstanding_tickets: u64::try_from(resume.outstanding_tickets)
                .unwrap_or(u64::MAX),
            clock_probes_sent: clock.probes_sent,
            clock_replies_sent: clock.replies_sent,
            clock_samples: clock.samples,
            clock_rejected: clock.rejected,
            clock_send_failures: clock.send_failures,
            ..Self::default()
        };
        if let Some(logger) = runtime.game_logger_snapshot() {
            out.logger_available = 1;
            out.logger_dropped = logger.dropped;
            out.logger_sink_panics = logger.sink_panics;
            if let Some(latency) = runtime.game_logger_callback_latency() {
                out.logger_callback_samples = latency.sample_count;
                out.logger_callback_p99_us = latency.p99_us;
                out.logger_callback_max_us = latency.max_us;
            }
        }
        out
    }
}

/// Runtime-wide staging gauges and cumulative counters. Forwarded means admitted to the
/// transport send queue, not delivered to the peer; no per-player labels or payloads appear.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRealtimeQueue {
    pub struct_size: u32,
    pub abi_version: u32,
    pub queued_messages: u64,
    pub queued_bytes: u64,
    pub admission_rejected: u64,
    pub replaced: u64,
    pub closed_dropped: u64,
    pub backpressure_dropped: u64,
    pub send_failed: u64,
    pub forwarded: u64,
}

impl Default for RnetGameRealtimeQueue {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            queued_messages: 0,
            queued_bytes: 0,
            admission_rejected: 0,
            replaced: 0,
            closed_dropped: 0,
            backpressure_dropped: 0,
            send_failed: 0,
            forwarded: 0,
        }
    }
}

impl From<RealtimeQueueSnapshot> for RnetGameRealtimeQueue {
    fn from(value: RealtimeQueueSnapshot) -> Self {
        Self {
            queued_messages: u64::try_from(value.queued_messages).unwrap_or(u64::MAX),
            queued_bytes: u64::try_from(value.queued_bytes).unwrap_or(u64::MAX),
            admission_rejected: value.admission_rejected,
            replaced: value.replaced,
            closed_dropped: value.closed_dropped,
            backpressure_dropped: value.backpressure_dropped,
            send_failed: value.send_failed,
            forwarded: value.forwarded,
            ..Self::default()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameBuffer {
    pub struct_size: u32,
    pub abi_version: u32,
    pub data: *const u8,
    pub len: usize,
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
