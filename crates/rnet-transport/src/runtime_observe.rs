//! Runtime address, event polling, and metrics query APIs.

use crate::kcp::KcpRetransmissionSnapshot;
use crate::metrics::{LatencyKind, LatencyMetricSnapshot, MetricsSnapshot, LATENCY_KIND_COUNT};
use crate::runtime::NetworkRuntime;
use crate::state::SessionTarget;
use rnet_core::{ErrorCode, Event, Handle, Lifecycle, Result, RnetError};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

impl NetworkRuntime {
    /// Returns authenticated-session KCP retransmissions; UDP/TCP have no such metric.
    pub fn kcp_retransmission_snapshot(
        &self,
        session: Handle,
    ) -> Result<Option<KcpRetransmissionSnapshot>> {
        let target = {
            let sessions = self.shared.sessions.lock().expect("session table poisoned");
            let route = sessions
                .get(session)
                .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?;
            if !route.established {
                return Err(RnetError::new(
                    ErrorCode::InvalidState,
                    "session is not established",
                ));
            }
            match &route.target {
                SessionTarget::Kcp { peer, .. } => Some((route.endpoint, *peer)),
                _ => None,
            }
        };
        Ok(target.map(|(endpoint, peer)| {
            self.shared
                .kcp_telemetry
                .snapshot(endpoint, peer)
                .unwrap_or_default()
        }))
    }

    pub fn endpoint_local_addr(&self, endpoint: Handle) -> Result<SocketAddr> {
        self.shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .get(endpoint)
            .map(|record| record.local_addr)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid endpoint"))
    }

    /// Returns the immutable transport selected when the endpoint was created.
    pub fn endpoint_transport(&self, endpoint: Handle) -> Result<rnet_core::Transport> {
        self.shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .get(endpoint)
            .map(|record| record.transport)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid endpoint"))
    }

    pub fn poll_events(&self, capacity: usize, timeout: Duration) -> Vec<Event> {
        let events = self.shared.events.poll(capacity, timeout);
        let now = Instant::now();
        for event in &events {
            self.shared.latencies.record(
                LatencyKind::EventQueue,
                now.saturating_duration_since(event.queued_at),
            );
        }
        events
    }

    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        let current_endpoints = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .len();
        let current_sessions = self
            .shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .len();
        self.shared.metrics.snapshot(
            self.shared.send_budget.used(),
            self.shared.send_budget.peak(),
            self.shared.events.queued_bytes(),
            current_endpoints,
            current_sessions,
        )
    }

    pub fn latency_snapshot(&self) -> [LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
        self.shared.latencies.snapshot()
    }

    /// Drains the latency interval accumulated since the previous window drain.
    pub fn drain_latency_window(&self) -> [LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
        self.shared.latencies.drain_window()
    }

    /// Renders dependency-free Prometheus exposition text for pull-based exporters.
    pub fn prometheus_snapshot(&self) -> String {
        let metrics = self.metrics_snapshot();
        let mut output = String::with_capacity(4096);
        macro_rules! counter {
            ($name:literal, $value:expr) => {
                output.push_str(concat!("# TYPE ", $name, " counter\n", $name, " "));
                output.push_str(&$value.to_string());
                output.push('\n');
            };
        }
        macro_rules! gauge {
            ($name:literal, $value:expr) => {
                output.push_str(concat!("# TYPE ", $name, " gauge\n", $name, " "));
                output.push_str(&$value.to_string());
                output.push('\n');
            };
        }
        counter!("rnet_frames_received_total", metrics.frames_received);
        counter!("rnet_frames_sent_total", metrics.frames_sent);
        counter!("rnet_bytes_received_total", metrics.bytes_received);
        counter!("rnet_bytes_sent_total", metrics.bytes_sent);
        counter!("rnet_events_dropped_total", metrics.events_dropped);
        counter!("rnet_send_would_block_total", metrics.send_would_block);
        counter!("rnet_protocol_errors_total", metrics.protocol_errors);
        counter!("rnet_admission_rejected_total", metrics.admission_rejected);
        output.push_str("# TYPE rnet_admission_rejected_reason_total counter\n");
        for (reason, count) in [
            "endpoint_limit",
            "endpoint_session_limit",
            "pending_handshake_limit",
            "ip_active_limit",
            "ip_rate_limit",
            "ip_table_limit",
            "kcp_peer_limit",
            "unauthenticated",
        ]
        .into_iter()
        .zip(metrics.admission_rejected_by_reason)
        {
            output.push_str(&format!(
                "rnet_admission_rejected_reason_total{{reason=\"{reason}\"}} {count}\n"
            ));
        }
        counter!(
            "rnet_lifecycle_events_rejected_total",
            metrics.lifecycle_events_rejected
        );
        gauge!("rnet_queued_send_bytes", metrics.queued_send_bytes);
        gauge!(
            "rnet_peak_queued_send_bytes",
            metrics.peak_queued_send_bytes
        );
        gauge!("rnet_queued_event_bytes", metrics.queued_event_bytes);
        gauge!("rnet_current_endpoints", metrics.current_endpoints);
        gauge!("rnet_current_sessions", metrics.current_sessions);
        gauge!("rnet_established_sessions", metrics.established_sessions);
        gauge!("rnet_pending_handshakes", metrics.pending_handshakes);
        gauge!(
            "rnet_peak_pending_handshakes",
            metrics.peak_pending_handshakes
        );
        for (index, count) in metrics.session_closed_by_reason.iter().enumerate() {
            let code = -(index as i32);
            output.push_str(&format!(
                "rnet_session_closed_total{{reason_code=\"{code}\"}} {count}\n"
            ));
        }
        for metric in self.latency_snapshot() {
            let kind = metric.kind as u32;
            let latency = metric.latency;
            for (quantile, value) in [
                ("0.5", latency.p50_us),
                ("0.9", latency.p90_us),
                ("0.95", latency.p95_us),
                ("0.99", latency.p99_us),
                ("0.999", latency.p999_us),
            ] {
                output.push_str(&format!(
                    "rnet_latency_us{{kind=\"{kind}\",quantile=\"{quantile}\"}} {value}\n"
                ));
            }
            output.push_str(&format!(
                "rnet_latency_samples_total{{kind=\"{kind}\"}} {}\n",
                latency.sample_count
            ));
            output.push_str(&format!(
                "rnet_latency_max_us{{kind=\"{kind}\"}} {}\n",
                latency.max_us
            ));
        }
        output
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.shared.state.load()
    }
}
