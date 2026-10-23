//! Game telemetry ABI layout and native snapshot conversion.

use crate::abi::RNET_ABI_VERSION;
use rnet_game::GameRuntime;
use std::mem::size_of;

/// Cumulative game-level counters; heartbeat RTT fields are cumulative histogram estimates.
/// The logger fields are meaningful only when `logger_available` is one. There are no
/// per-session labels, player identifiers, credentials, or business payloads in this structure.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Cumulative game telemetry and current gauges; independent counters can race a snapshot.
pub struct RnetGameMetrics {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// 0 logging disabled, 1 logger installed; distinguishes no logger from zero drops.
    pub logger_available: u32,
    /// Reserved output, written as zero by the library.
    pub reserved: u32,
    /// Cumulative protected probes admitted locally, not confirmed delivered.
    pub heartbeat_probes_sent: u64,
    /// Cumulative failed local probe sends.
    pub heartbeat_probe_send_failures: u64,
    /// Cumulative protected replies admitted locally.
    pub heartbeat_replies_sent: u64,
    /// Cumulative failed local reply sends.
    pub heartbeat_reply_send_failures: u64,
    /// Cumulative timely replies matching outstanding local challenges.
    pub heartbeat_replies_matched: u64,
    /// Cumulative invalid/late challenge replies; do not refresh liveness.
    pub heartbeat_replies_rejected: u64,
    /// Cumulative inbound probes suppressed by response rate limits.
    pub heartbeat_probes_rate_limited: u64,
    /// Cumulative heartbeat deadlines causing session-close requests.
    pub heartbeat_timeouts: u64,
    /// Cumulative accepted RTT samples; zero means unavailable.
    pub heartbeat_rtt_samples: u64,
    /// Cumulative estimated heartbeat RTT median in microseconds.
    pub heartbeat_rtt_p50_us: u64,
    /// Cumulative estimated heartbeat RTT P90 in microseconds.
    pub heartbeat_rtt_p90_us: u64,
    /// Cumulative estimated heartbeat RTT P95 in microseconds.
    pub heartbeat_rtt_p95_us: u64,
    /// Cumulative estimated heartbeat RTT P99 in microseconds.
    pub heartbeat_rtt_p99_us: u64,
    /// Cumulative estimated heartbeat RTT P99.9 in microseconds.
    pub heartbeat_rtt_p999_us: u64,
    /// Maximum sampled heartbeat RTT in microseconds.
    pub heartbeat_rtt_max_us: u64,
    /// Cumulative tickets issued and admitted to protected send, not confirmed delivered.
    pub resume_tickets_issued: u64,
    /// Cumulative decoded resume joins reaching ticket validation.
    pub resume_requests_received: u64,
    /// Cumulative invalid, expired, replayed or incompatible ticket claims.
    pub resume_tickets_rejected: u64,
    /// Cumulative valid resume claims denied by business authorization.
    pub resume_authorization_denied: u64,
    /// Cumulative pending/in-flight resume claims revoked by explicit cleanup.
    pub resume_pending_revoked: u64,
    /// Cumulative completed resume publications on this runtime.
    pub resume_sessions_resumed: u64,
    /// Current retained registry entries; expiry cleanup is opportunistic.
    pub resume_outstanding_tickets: u64,
    /// Cumulative locally admitted client clock probes.
    pub clock_probes_sent: u64,
    /// Cumulative locally admitted server clock replies.
    pub clock_replies_sent: u64,
    /// Cumulative accepted client clock samples.
    pub clock_samples: u64,
    /// Cumulative rate-limited probes or tracker-rejected replies, not every parser error.
    pub clock_rejected: u64,
    /// Cumulative probe generation/admission and reply send-admission failures.
    pub clock_send_failures: u64,
    /// Cumulative records rejected by bounded logger queues.
    pub logger_dropped: u64,
    /// Cumulative caught sink panics; do not throw foreign exceptions through the callback.
    pub logger_sink_panics: u64,
    /// Completed callback samples, including caught panics.
    pub logger_callback_samples: u64,
    /// Cumulative estimated callback execution P99 in microseconds; excludes queue wait.
    pub logger_callback_p99_us: u64,
    /// Largest measured callback execution time in microseconds.
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
