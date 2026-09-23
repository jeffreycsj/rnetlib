//! Poll-driven, bounded cumulative summaries shared by every language facade.

use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Result, RnetError};
use rnet_observe::{LatencySnapshot, LogLevel, LogRecord};
use rnet_transport::LatencyKind;
use std::fmt::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

impl GameRuntime {
    /// Sets the poll-driven summary interval (default 30 seconds); zero disables summaries.
    /// Histograms are cumulative, and querying/logging never drains a caller's interval window.
    pub fn set_metrics_log_interval(&self, interval: Duration) -> Result<()> {
        let now = Instant::now();
        if now.checked_add(interval).is_none() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "log interval exceeds monotonic clock range",
            ));
        }
        *self
            .diagnostics
            .summary
            .lock()
            .expect("summary schedule poisoned") = (interval, now);
        Ok(())
    }

    pub(crate) fn maybe_log_metrics(&self) {
        let Some(logger) = self.diagnostics.logger.as_ref() else {
            return;
        };
        {
            let mut schedule = self
                .diagnostics
                .summary
                .lock()
                .expect("summary schedule poisoned");
            let now = Instant::now();
            if schedule.0.is_zero() || now.saturating_duration_since(schedule.1) < schedule.0 {
                return;
            }
            schedule.1 = now;
        }
        let mut message = String::from("scope=cumulative");
        append_latency(
            &mut message,
            "heartbeat_rtt",
            self.heartbeat_metrics_snapshot().rtt,
        );
        for metric in self.network.latency_snapshot() {
            let name = match metric.kind {
                LatencyKind::Connect => "connect",
                LatencyKind::CryptoHandshake => "crypto_handshake",
                LatencyKind::AuthWait => "auth_wait",
                LatencyKind::SendQueue => "send_queue",
                LatencyKind::EventQueue => "event_queue",
                LatencyKind::KcpRtt => "kcp_rtt",
                LatencyKind::KcpUpdateDelay => "kcp_update_delay",
                LatencyKind::LoggerCallback => continue,
            };
            append_latency(&mut message, name, metric.latency);
        }
        let queue = self.scheduled_queue_snapshot();
        append_latency(&mut message, "scheduled_queue", queue.queue_delay);
        append_latency(&mut message, "logger_callback", logger.callback_latency());
        let _ = write!(
            message,
            " queued_messages={} queued_bytes={} logger_dropped={} logger_panics={}",
            queue.queued_messages,
            queue.queued_bytes,
            logger.dropped(),
            logger.sink_panics()
        );
        let _ = logger.log(LogRecord {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            level: LogLevel::Info,
            event_name: "game_latency_summary".into(),
            target: "rnet.game".into(),
            message,
            runtime: self.diagnostics.runtime_id,
            endpoint: 0,
            session: 0,
            transport: 0,
            error_code: ErrorCode::Ok,
            correlation_id: 0,
        });
    }
}

fn append_latency(message: &mut String, kind: &str, value: LatencySnapshot) {
    let _ = write!(
        message,
        " kind={kind} count={} p90_us={} p95_us={} p99_us={} p999_us={} max_us={}",
        value.sample_count, value.p90_us, value.p95_us, value.p99_us, value.p999_us, value.max_us
    );
}
