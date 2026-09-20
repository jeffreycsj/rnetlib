//! Game-facing access to the existing bounded transport metrics and latency windows.

use crate::runtime::GameRuntime;
use rnet_transport::{LatencyMetricSnapshot, MetricsSnapshot, LATENCY_KIND_COUNT};

impl GameRuntime {
    /// Returns cumulative counters and current resource gauges.
    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.network.metrics_snapshot()
    }

    /// Returns dependency-free Prometheus exposition for the underlying transport runtime.
    pub fn prometheus_snapshot(&self) -> String {
        self.network.prometheus_snapshot()
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
