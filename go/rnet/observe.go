package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"time"
)

// Metrics combines runtime-wide cumulative counters and instantaneous queue gauges.
// Concurrent updates mean fields are not one atomic snapshot; reads do not reset counters.
type Metrics struct {
	// FramesReceived cumulative logical frames received, not raw IP packets.
	FramesReceived uint64
	// FramesSent cumulative logical frames accepted for transport sending, not remote acknowledgements.
	FramesSent uint64
	// BytesReceived cumulative decoded frame body bytes received; excludes outer framing.
	BytesReceived uint64
	// BytesSent cumulative logical encoded frame bytes sent; includes logical framing.
	BytesSent uint64
	// EventsDropped cumulative transport event queue admission failures.
	EventsDropped uint64
	// SendWouldBlock cumulative transport send backpressure rejections.
	SendWouldBlock uint64
	// ProtocolErrors cumulative malformed/invalid transport-protocol input observations.
	ProtocolErrors uint64
	// LifecycleEventsRejected cumulative failed lifecycle-event admissions; cleanup does not depend on delivery.
	LifecycleEventsRejected uint64
	// AdmissionRejected cumulative endpoint/session/handshake admission failures.
	AdmissionRejected uint64
	// QueuedSendBytes current send byte reservations; includes framing, not total process RSS.
	QueuedSendBytes uint64
	// PeakQueuedSendBytes highest runtime send byte reservation observed.
	PeakQueuedSendBytes uint64
	// QueuedEventBytes current payload bytes awaiting transport poll; excludes exported buffer tokens.
	QueuedEventBytes uint64
	// CurrentEndpoints current registered endpoints, including listeners and client connections.
	CurrentEndpoints uint64
	// CurrentSessions current registered sessions including unfinished handshakes.
	CurrentSessions uint64
	// EstablishedSessions current transport-open sessions; not proof of game readiness.
	EstablishedSessions uint64
	// PendingHandshakes current sessions awaiting transport handshake/authorization.
	PendingHandshakes uint64
	// PeakPendingHandshakes highest pending-handshake count observed.
	PeakPendingHandshakes uint64
	// AdmissionRejectedByReason counts: endpoint, endpoint-session, pending, IP-active, IP-rate, IP-table, KCP-peer, unauthenticated.
	AdmissionRejectedByReason [8]uint64
	// SessionClosedByReason cumulative closes indexed by negated error code, 0..18 (zero maps to 0).
	SessionClosedByReason [19]uint64
	// LogsDropped cumulative bounded logger admission drops; zero also when no logger is installed.
	LogsDropped uint64
	// LoggerPanics cumulative caught Rust logger panics; foreign exceptions must never cross the C boundary.
	LoggerPanics uint64
}

// LatencyMetric summarizes one operation's histogram. Durations have native microsecond
// precision; SampleCount zero means unavailable, not zero latency. Percentiles are estimates.
type LatencyMetric struct {
	// Kind identifies the sampled operation; duration fields use native microsecond precision.
	Kind LatencyKind
	// SampleCount samples represented; zero means unavailable, not measured zero latency.
	SampleCount uint64
	// P50 estimated median latency; histogram bucket approximation.
	P50 time.Duration
	// P90 estimated 90th percentile latency.
	P90 time.Duration
	// P95 estimated 95th percentile latency.
	P95 time.Duration
	// P99 estimated 99th percentile latency.
	P99 time.Duration
	// P999 estimated 99.9th percentile latency.
	P999 time.Duration
	// Max largest sampled duration; concurrent snapshots are not atomic across all fields.
	Max time.Duration
}

// Metrics returns a copied runtime-wide transport snapshot without resetting counters.
func (r *Runtime) Metrics() (Metrics, error) {
	defer lockNativeThread()()
	var result Metrics
	handle, err := r.handleValue()
	if err != nil {
		return result, err
	}
	var native C.rnet_metrics_t
	if err := statusError(C.rnet_metrics_snapshot(handle, &native)); err != nil {
		return result, err
	}
	result = Metrics{
		FramesReceived: uint64(native.frames_received), FramesSent: uint64(native.frames_sent),
		BytesReceived: uint64(native.bytes_received), BytesSent: uint64(native.bytes_sent),
		EventsDropped: uint64(native.events_dropped), SendWouldBlock: uint64(native.send_would_block),
		ProtocolErrors: uint64(native.protocol_errors), LogsDropped: uint64(native.logs_dropped),
		LifecycleEventsRejected: uint64(native.lifecycle_events_rejected),
		AdmissionRejected:       uint64(native.admission_rejected),
		QueuedSendBytes:         uint64(native.queued_send_bytes),
		PeakQueuedSendBytes:     uint64(native.peak_queued_send_bytes),
		QueuedEventBytes:        uint64(native.queued_event_bytes),
		CurrentEndpoints:        uint64(native.current_endpoints),
		CurrentSessions:         uint64(native.current_sessions),
		EstablishedSessions:     uint64(native.established_sessions),
		PendingHandshakes:       uint64(native.pending_handshakes),
		PeakPendingHandshakes:   uint64(native.peak_pending_handshakes),
		LoggerPanics:            uint64(native.logger_panics),
	}
	for index := range result.SessionClosedByReason {
		result.SessionClosedByReason[index] = uint64(native.session_closed_by_reason[index])
	}
	for index := range result.AdmissionRejectedByReason {
		result.AdmissionRejectedByReason[index] = uint64(native.admission_rejected_by_reason[index])
	}
	return result, nil
}

// LatencyMetrics returns cumulative per-kind histograms without rotating them.
// Check SampleCount before interpreting P95/P99; these are not all end-to-end message latencies.
func (r *Runtime) LatencyMetrics() ([]LatencyMetric, error) {
	return r.latencyMetrics(false)
}

// DrainLatencyMetrics reads and rotates per-kind interval histograms.
// Kinds rotate independently, not as one atomic window; concurrent consumers split samples.
func (r *Runtime) DrainLatencyMetrics() ([]LatencyMetric, error) {
	return r.latencyMetrics(true)
}

func (r *Runtime) latencyMetrics(drain bool) ([]LatencyMetric, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return nil, err
	}
	var count C.size_t
	drainWindow := C.uint32_t(0)
	if drain {
		drainWindow = 1
	}
	if err := statusError(C.rnet_latency_snapshot(handle, nil, 0, &count, drainWindow)); err != nil {
		return nil, err
	}
	if count == 0 {
		return nil, nil
	}
	native := make([]C.rnet_latency_metric_t, int(count))
	if err := statusError(C.rnet_latency_snapshot(handle, &native[0], count, &count, drainWindow)); err != nil {
		return nil, err
	}
	result := make([]LatencyMetric, 0, int(count))
	for _, metric := range native[:int(count)] {
		result = append(result, LatencyMetric{
			Kind: LatencyKind(metric.kind), SampleCount: uint64(metric.sample_count),
			P50: microseconds(metric.p50_us), P90: microseconds(metric.p90_us),
			P95: microseconds(metric.p95_us), P99: microseconds(metric.p99_us),
			P999: microseconds(metric.p999_us),
			Max:  microseconds(metric.max_us),
		})
	}
	return result, nil
}

// SetMetricsLogInterval changes automatic transport summary logging.
// interval is truncated to whole milliseconds: zero disables, a negative converted
// value or uint32-millisecond overflow fails. Install a logger to receive summaries.
func (r *Runtime) SetMetricsLogInterval(interval time.Duration) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	milliseconds := interval.Milliseconds()
	if milliseconds < 0 {
		return fmt.Errorf("rnet: metrics log interval must not be negative")
	}
	return statusError(C.rnet_metrics_log_interval_set(handle, C.uint64_t(milliseconds)))
}
