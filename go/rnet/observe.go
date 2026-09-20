package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"time"
)

type Metrics struct {
	FramesReceived            uint64
	FramesSent                uint64
	BytesReceived             uint64
	BytesSent                 uint64
	EventsDropped             uint64
	SendWouldBlock            uint64
	ProtocolErrors            uint64
	LifecycleEventsRejected   uint64
	AdmissionRejected         uint64
	QueuedSendBytes           uint64
	PeakQueuedSendBytes       uint64
	QueuedEventBytes          uint64
	CurrentEndpoints          uint64
	CurrentSessions           uint64
	EstablishedSessions       uint64
	PendingHandshakes         uint64
	PeakPendingHandshakes     uint64
	AdmissionRejectedByReason [8]uint64
	SessionClosedByReason     [19]uint64
	LogsDropped               uint64
	LoggerPanics              uint64
}

type LatencyMetric struct {
	Kind        LatencyKind
	SampleCount uint64
	P50         time.Duration
	P90         time.Duration
	P95         time.Duration
	P99         time.Duration
	P999        time.Duration
	Max         time.Duration
}

func (r *Runtime) Metrics() (Metrics, error) {
	var result Metrics
	handle, err := r.handleValue()
	if err != nil {
		return result, err
	}
	var native C.rnet_metrics_v3_t
	if err := statusError(C.rnet_metrics_snapshot_v3(handle, &native)); err != nil {
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

func (r *Runtime) LatencyMetrics() ([]LatencyMetric, error) {
	return r.latencyMetrics(false)
}

// DrainLatencyMetrics returns and rotates the current interval histogram.
func (r *Runtime) DrainLatencyMetrics() ([]LatencyMetric, error) {
	return r.latencyMetrics(true)
}

func (r *Runtime) latencyMetrics(drain bool) ([]LatencyMetric, error) {
	handle, err := r.handleValue()
	if err != nil {
		return nil, err
	}
	var count C.size_t
	drainWindow := C.uint32_t(0)
	if drain {
		drainWindow = 1
	}
	if err := statusError(C.rnet_latency_snapshot_v2(handle, nil, 0, &count, drainWindow)); err != nil {
		return nil, err
	}
	if count == 0 {
		return nil, nil
	}
	native := make([]C.rnet_latency_metric_v2_t, int(count))
	if err := statusError(C.rnet_latency_snapshot_v2(handle, &native[0], count, &count, drainWindow)); err != nil {
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

func (r *Runtime) SetMetricsLogInterval(interval time.Duration) error {
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
