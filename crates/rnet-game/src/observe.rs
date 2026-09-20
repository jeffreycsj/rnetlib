//! Game-facing transport and authenticated-heartbeat telemetry.

use crate::runtime::GameRuntime;
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

    /// Returns transport and game-heartbeat Prometheus metrics without per-session labels.
    pub fn prometheus_snapshot(&self) -> String {
        let mut output = self.network.prometheus_snapshot();
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
