//! Game-facing transport and authenticated-heartbeat telemetry.

use crate::runtime::GameRuntime;
use crate::RangeBufferSnapshot;
use rnet_observe::{LatencyHistogram, LatencySnapshot};
use rnet_transport::{LatencyMetricSnapshot, MetricsSnapshot, LATENCY_KIND_COUNT};
use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Cumulative, process-local heartbeat telemetry. Snapshots taken during concurrent I/O may
/// observe counters and histogram buckets at slightly different instants.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HeartbeatMetricsSnapshot {
    pub probes_sent: u64,
    pub probe_send_failures: u64,
    pub replies_sent: u64,
    pub reply_send_failures: u64,
    pub replies_matched: u64,
    pub replies_rejected: u64,
    pub probes_rate_limited: u64,
    pub timeouts: u64,
    /// Only authenticated, challenge-matched replies arriving before their deadline are sampled.
    pub rtt: LatencySnapshot,
}

/// Low-cardinality, cumulative resume telemetry. No ticket or player identifier is exported.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResumeMetricsSnapshot {
    pub tickets_issued: u64,
    pub requests_received: u64,
    pub tickets_rejected: u64,
    pub authorization_denied: u64,
    pub pending_revoked: u64,
    pub sessions_resumed: u64,
    pub outstanding_tickets: usize,
}

/// Cumulative protocol-input counters. Plaintext corruption is intentionally aggregated instead
/// of being logged or labeled by session, because an unauthenticated sender controls its rate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProtocolMetricsSnapshot {
    pub plaintext_invalid_envelopes_dropped: u64,
}

#[derive(Default)]
pub(crate) struct ProtocolMetrics {
    pub(crate) plaintext_invalid_envelopes_dropped: AtomicU64,
}

impl ProtocolMetrics {
    fn snapshot(&self) -> ProtocolMetricsSnapshot {
        ProtocolMetricsSnapshot {
            plaintext_invalid_envelopes_dropped: self
                .plaintext_invalid_envelopes_dropped
                .load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
pub(crate) struct ResumeMetrics {
    pub(crate) tickets_issued: AtomicU64,
    pub(crate) requests_received: AtomicU64,
    pub(crate) tickets_rejected: AtomicU64,
    pub(crate) authorization_denied: AtomicU64,
    pub(crate) pending_revoked: AtomicU64,
    pub(crate) sessions_resumed: AtomicU64,
}

impl ResumeMetrics {
    fn snapshot(&self, outstanding_tickets: usize) -> ResumeMetricsSnapshot {
        ResumeMetricsSnapshot {
            tickets_issued: self.tickets_issued.load(Ordering::Relaxed),
            requests_received: self.requests_received.load(Ordering::Relaxed),
            tickets_rejected: self.tickets_rejected.load(Ordering::Relaxed),
            authorization_denied: self.authorization_denied.load(Ordering::Relaxed),
            pending_revoked: self.pending_revoked.load(Ordering::Relaxed),
            sessions_resumed: self.sessions_resumed.load(Ordering::Relaxed),
            outstanding_tickets,
        }
    }
}

#[derive(Default)]
pub(crate) struct HeartbeatMetrics {
    pub(crate) probes_sent: AtomicU64,
    pub(crate) probe_send_failures: AtomicU64,
    pub(crate) replies_sent: AtomicU64,
    pub(crate) reply_send_failures: AtomicU64,
    pub(crate) replies_matched: AtomicU64,
    pub(crate) replies_rejected: AtomicU64,
    pub(crate) probes_rate_limited: AtomicU64,
    pub(crate) timeouts: AtomicU64,
    rtt_cumulative: LatencyHistogram,
    rtt_window: LatencyHistogram,
}

impl HeartbeatMetrics {
    pub(crate) fn record_matched_rtt(&self, rtt: Duration) {
        self.rtt_cumulative.record(rtt);
        self.rtt_window.record(rtt);
        self.replies_matched.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> HeartbeatMetricsSnapshot {
        HeartbeatMetricsSnapshot {
            probes_sent: self.probes_sent.load(Ordering::Relaxed),
            probe_send_failures: self.probe_send_failures.load(Ordering::Relaxed),
            replies_sent: self.replies_sent.load(Ordering::Relaxed),
            reply_send_failures: self.reply_send_failures.load(Ordering::Relaxed),
            replies_matched: self.replies_matched.load(Ordering::Relaxed),
            replies_rejected: self.replies_rejected.load(Ordering::Relaxed),
            probes_rate_limited: self.probes_rate_limited.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            rtt: self.rtt_cumulative.snapshot(),
        }
    }
}

impl GameRuntime {
    pub fn protocol_metrics_snapshot(&self) -> ProtocolMetricsSnapshot {
        self.protocol_metrics.snapshot()
    }

    pub fn resume_metrics_snapshot(&self) -> ResumeMetricsSnapshot {
        let outstanding = self
            .resume
            .lock()
            .expect("resume state poisoned")
            .tickets
            .outstanding();
        self.resume_metrics.snapshot(outstanding)
    }

    /// Returns cumulative heartbeat counters and RTT percentiles across this runtime's sessions.
    pub fn heartbeat_metrics_snapshot(&self) -> HeartbeatMetricsSnapshot {
        self.heartbeat_metrics.snapshot()
    }

    /// Rotates only the heartbeat RTT monitoring window; cumulative counters stay intact.
    pub fn drain_heartbeat_rtt_window(&self) -> LatencySnapshot {
        self.heartbeat_metrics.rtt_window.snapshot_and_reset()
    }

    /// Returns cumulative counters and current resource gauges.
    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.network.metrics_snapshot()
    }

    /// Returns current/peak wire-v4 early-data usage and cumulative admission failures.
    pub fn range_buffer_snapshot(&self) -> RangeBufferSnapshot {
        self.range.lock().expect("range state poisoned").snapshot()
    }

    /// Returns transport and game-heartbeat Prometheus metrics without per-session labels.
    pub fn prometheus_snapshot(&self) -> String {
        let mut output = self.network.prometheus_snapshot();
        let latest = self.transport_latest_snapshot();
        for (name, value) in [
            (
                "rnet_game_transport_latest_replaced_total",
                latest.pending_replaced,
            ),
            (
                "rnet_game_transport_latest_worker_pickups_total",
                latest.worker_pickups,
            ),
            (
                "rnet_game_transport_latest_admission_would_block_total",
                latest.admission_would_block,
            ),
            (
                "rnet_game_transport_latest_admission_invalid_handle_total",
                latest.admission_invalid_handle,
            ),
            (
                "rnet_game_transport_latest_admission_invalid_state_total",
                latest.admission_invalid_state,
            ),
            (
                "rnet_game_transport_latest_admission_handshake_required_total",
                latest.admission_handshake_required,
            ),
            (
                "rnet_game_transport_latest_admission_not_supported_total",
                latest.admission_not_supported,
            ),
            (
                "rnet_game_transport_latest_admission_message_too_large_total",
                latest.admission_message_too_large,
            ),
            (
                "rnet_game_transport_latest_admission_other_failures_total",
                latest.admission_other_failures,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        let metrics = self.heartbeat_metrics_snapshot();
        for (name, value) in [
            ("rnet_game_heartbeat_probes_sent_total", metrics.probes_sent),
            (
                "rnet_game_heartbeat_probe_send_failures_total",
                metrics.probe_send_failures,
            ),
            (
                "rnet_game_heartbeat_replies_sent_total",
                metrics.replies_sent,
            ),
            (
                "rnet_game_heartbeat_reply_send_failures_total",
                metrics.reply_send_failures,
            ),
            (
                "rnet_game_heartbeat_replies_matched_total",
                metrics.replies_matched,
            ),
            (
                "rnet_game_heartbeat_replies_rejected_total",
                metrics.replies_rejected,
            ),
            (
                "rnet_game_heartbeat_probes_rate_limited_total",
                metrics.probes_rate_limited,
            ),
            ("rnet_game_heartbeat_timeouts_total", metrics.timeouts),
            (
                "rnet_game_heartbeat_rtt_samples_total",
                metrics.rtt.sample_count,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        output.push_str("# TYPE rnet_game_heartbeat_rtt_us gauge\n");
        for (quantile, value) in [
            ("0.5", metrics.rtt.p50_us),
            ("0.9", metrics.rtt.p90_us),
            ("0.95", metrics.rtt.p95_us),
            ("0.99", metrics.rtt.p99_us),
            ("0.999", metrics.rtt.p999_us),
        ] {
            let _ = writeln!(
                output,
                "rnet_game_heartbeat_rtt_us{{quantile=\"{quantile}\"}} {value}"
            );
        }
        let _ = writeln!(
            output,
            "# TYPE rnet_game_heartbeat_rtt_max_us gauge\nrnet_game_heartbeat_rtt_max_us {}",
            metrics.rtt.max_us
        );
        let clock = self.clock_sync_metrics_snapshot();
        for (name, value) in [
            ("rnet_game_clock_probes_sent_total", clock.probes_sent),
            ("rnet_game_clock_replies_sent_total", clock.replies_sent),
            ("rnet_game_clock_samples_total", clock.samples),
            ("rnet_game_clock_rejected_total", clock.rejected),
            ("rnet_game_clock_send_failures_total", clock.send_failures),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        let protocol = self.protocol_metrics_snapshot();
        let _ = writeln!(
            output,
            "# TYPE rnet_game_plaintext_invalid_envelopes_dropped_total counter\nrnet_game_plaintext_invalid_envelopes_dropped_total {}",
            protocol.plaintext_invalid_envelopes_dropped
        );
        let realtime = self.realtime_queue_snapshot();
        for (name, value) in [
            (
                "rnet_game_realtime_admission_rejected_total",
                realtime.admission_rejected,
            ),
            ("rnet_game_realtime_replaced_total", realtime.replaced),
            (
                "rnet_game_realtime_closed_dropped_total",
                realtime.closed_dropped,
            ),
            (
                "rnet_game_realtime_backpressure_dropped_total",
                realtime.backpressure_dropped,
            ),
            ("rnet_game_realtime_send_failed_total", realtime.send_failed),
            ("rnet_game_realtime_forwarded_total", realtime.forwarded),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        for (name, value) in [
            (
                "rnet_game_realtime_queued_messages",
                realtime.queued_messages,
            ),
            ("rnet_game_realtime_queued_bytes", realtime.queued_bytes),
        ] {
            let _ = writeln!(output, "# TYPE {name} gauge\n{name} {value}");
        }
        let scheduled = self.scheduled_queue_snapshot();
        for (name, value) in [
            (
                "rnet_game_scheduled_admission_rejected_total",
                scheduled.admission_rejected,
            ),
            (
                "rnet_game_scheduled_expired_dropped_total",
                scheduled.expired_dropped,
            ),
            (
                "rnet_game_scheduled_closed_dropped_total",
                scheduled.closed_dropped,
            ),
            (
                "rnet_game_scheduled_send_failed_total",
                scheduled.send_failed,
            ),
            ("rnet_game_scheduled_forwarded_total", scheduled.forwarded),
            (
                "rnet_game_scheduled_backpressure_requeued_total",
                scheduled.backpressure_requeued,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        for (name, value) in [
            (
                "rnet_game_scheduled_queued_messages",
                scheduled.queued_messages,
            ),
            ("rnet_game_scheduled_queued_bytes", scheduled.queued_bytes),
        ] {
            let _ = writeln!(output, "# TYPE {name} gauge\n{name} {value}");
        }
        output.push_str("# TYPE rnet_game_scheduled_admitted_by_priority_total counter\n");
        output.push_str("# TYPE rnet_game_scheduled_forwarded_by_priority_total counter\n");
        for (index, priority) in ["low", "normal", "high", "critical"].iter().enumerate() {
            let _ = writeln!(
                output,
                "rnet_game_scheduled_admitted_by_priority_total{{priority=\"{priority}\"}} {}",
                scheduled.admitted_by_priority[index]
            );
            let _ = writeln!(
                output,
                "rnet_game_scheduled_forwarded_by_priority_total{{priority=\"{priority}\"}} {}",
                scheduled.forwarded_by_priority[index]
            );
        }
        for (metric, latency) in [
            ("rnet_game_scheduled_queue_delay_us", scheduled.queue_delay),
            (
                "rnet_game_scheduled_tick_queue_delay_us",
                scheduled.tick_queue_delay,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {metric} gauge");
            for (quantile, value) in [
                ("0.5", latency.p50_us),
                ("0.9", latency.p90_us),
                ("0.95", latency.p95_us),
                ("0.99", latency.p99_us),
                ("0.999", latency.p999_us),
            ] {
                let _ = writeln!(output, "{metric}{{quantile=\"{quantile}\"}} {value}");
            }
            let _ = writeln!(output, "{metric}{{quantile=\"max\"}} {}", latency.max_us);
        }
        let _ = writeln!(
            output,
            "# TYPE rnet_game_scheduled_queue_delay_samples_total counter\nrnet_game_scheduled_queue_delay_samples_total {}",
            scheduled.queue_delay.sample_count
        );
        let _ = writeln!(
            output,
            "# TYPE rnet_game_scheduled_tick_queue_delay_samples_total counter\nrnet_game_scheduled_tick_queue_delay_samples_total {}",
            scheduled.tick_queue_delay.sample_count
        );
        let range = self.range_buffer_snapshot();
        for (name, value) in [
            (
                "rnet_game_range_early_session_admission_rejected_total",
                range.session_admission_rejected,
            ),
            (
                "rnet_game_range_early_runtime_admission_rejected_total",
                range.runtime_admission_rejected,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        for (name, value) in [
            (
                "rnet_game_range_early_buffered_messages",
                range.buffered_messages,
            ),
            ("rnet_game_range_early_buffered_bytes", range.buffered_bytes),
            (
                "rnet_game_range_early_peak_buffered_messages",
                range.peak_buffered_messages,
            ),
            (
                "rnet_game_range_early_peak_buffered_bytes",
                range.peak_buffered_bytes,
            ),
            (
                "rnet_game_range_early_max_buffered_messages",
                range.max_buffered_messages,
            ),
            (
                "rnet_game_range_early_max_buffered_bytes",
                range.max_buffered_bytes,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} gauge\n{name} {value}");
        }
        let resume = self.resume_metrics_snapshot();
        for (name, value) in [
            (
                "rnet_game_resume_tickets_issued_total",
                resume.tickets_issued,
            ),
            (
                "rnet_game_resume_requests_received_total",
                resume.requests_received,
            ),
            (
                "rnet_game_resume_tickets_rejected_total",
                resume.tickets_rejected,
            ),
            (
                "rnet_game_resume_authorization_denied_total",
                resume.authorization_denied,
            ),
            (
                "rnet_game_resume_pending_revoked_total",
                resume.pending_revoked,
            ),
            (
                "rnet_game_resume_sessions_resumed_total",
                resume.sessions_resumed,
            ),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        let _ = writeln!(
            output,
            "# TYPE rnet_game_resume_outstanding_tickets gauge\nrnet_game_resume_outstanding_tickets {}",
            resume.outstanding_tickets
        );
        let logger = self.game_logger_snapshot();
        let _ = writeln!(
            output,
            "# TYPE rnet_game_logger_enabled gauge\nrnet_game_logger_enabled {}",
            u8::from(logger.is_some())
        );
        let logger = logger.unwrap_or_default();
        for (name, value) in [
            ("rnet_game_logger_dropped_total", logger.dropped),
            ("rnet_game_logger_sink_panics_total", logger.sink_panics),
        ] {
            let _ = writeln!(output, "# TYPE {name} counter\n{name} {value}");
        }
        output
    }

    /// Returns cumulative P50/P90/P95/P99/P99.9/max latency summaries.
    pub fn latency_snapshot(&self) -> [LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
        self.network.latency_snapshot()
    }

    /// Atomically rotates the interval latency window for periodic game telemetry.
    pub fn drain_latency_window(&self) -> [LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
        self.network.drain_latency_window()
    }
}
