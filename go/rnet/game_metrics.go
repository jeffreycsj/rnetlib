package rnet

/*
#include "native.h"
*/
import "C"

import "time"

// GameRealtimeQueue reports runtime-wide LatestOnly staging; Forwarded means
// admitted to the transport queue, not delivered to the peer.
type GameRealtimeQueue struct {
	QueuedMessages      uint64
	QueuedBytes         uint64
	AdmissionRejected   uint64
	Replaced            uint64
	ClosedDropped       uint64
	BackpressureDropped uint64
	SendFailed          uint64
	Forwarded           uint64
}

// GameMetrics is cumulative, process-local telemetry without player labels.
// LoggerAvailable distinguishes disabled logging from zero dropped records.
type GameMetrics struct {
	LoggerAvailable bool
	Heartbeat       GameHeartbeatMetrics
	Resume          GameResumeMetrics
	Clock           GameClockMetrics
	Logger          GameLoggerMetrics
}

type GameHeartbeatMetrics struct {
	ProbesSent, ProbeSendFailures, RepliesSent, ReplySendFailures uint64
	RepliesMatched, RepliesRejected, ProbesRateLimited, Timeouts  uint64
	RTTSamples                                                    uint64
	RTTP50, RTTP90, RTTP95, RTTP99, RTTP999, RTTMax               time.Duration
}

type GameResumeMetrics struct {
	TicketsIssued, RequestsReceived, TicketsRejected                         uint64
	AuthorizationDenied, PendingRevoked, SessionsResumed, OutstandingTickets uint64
}

type GameClockMetrics struct {
	ProbesSent, RepliesSent, Samples, Rejected, SendFailures uint64
}

type GameLoggerMetrics struct {
	Dropped, SinkPanics, CallbackSamples uint64
	CallbackP99, CallbackMax             time.Duration
}

// MetricsSnapshot exposes the same cumulative game counters as the Rust facade.
func (r *GameRuntime) MetricsSnapshot() (GameMetrics, error) {
	handle, err := r.handleValue()
	if err != nil {
		return GameMetrics{}, err
	}
	var raw C.rnet_game_metrics_t
	if err := statusError(C.rnet_game_metrics_snapshot(handle, &raw)); err != nil {
		return GameMetrics{}, err
	}
	metrics := GameMetrics{
		LoggerAvailable: raw.logger_available != 0,
		Heartbeat: GameHeartbeatMetrics{
			ProbesSent: uint64(raw.heartbeat_probes_sent), ProbeSendFailures: uint64(raw.heartbeat_probe_send_failures),
			RepliesSent: uint64(raw.heartbeat_replies_sent), ReplySendFailures: uint64(raw.heartbeat_reply_send_failures),
			RepliesMatched: uint64(raw.heartbeat_replies_matched), RepliesRejected: uint64(raw.heartbeat_replies_rejected),
			ProbesRateLimited: uint64(raw.heartbeat_probes_rate_limited), Timeouts: uint64(raw.heartbeat_timeouts),
			RTTSamples: uint64(raw.heartbeat_rtt_samples),
			RTTP50:     microseconds(raw.heartbeat_rtt_p50_us), RTTP90: microseconds(raw.heartbeat_rtt_p90_us),
			RTTP95: microseconds(raw.heartbeat_rtt_p95_us), RTTP99: microseconds(raw.heartbeat_rtt_p99_us),
			RTTP999: microseconds(raw.heartbeat_rtt_p999_us), RTTMax: microseconds(raw.heartbeat_rtt_max_us),
		},
		Resume: GameResumeMetrics{
			TicketsIssued: uint64(raw.resume_tickets_issued), RequestsReceived: uint64(raw.resume_requests_received),
			TicketsRejected: uint64(raw.resume_tickets_rejected), AuthorizationDenied: uint64(raw.resume_authorization_denied),
			PendingRevoked: uint64(raw.resume_pending_revoked), SessionsResumed: uint64(raw.resume_sessions_resumed),
			OutstandingTickets: uint64(raw.resume_outstanding_tickets),
		},
		Clock: GameClockMetrics{
			ProbesSent: uint64(raw.clock_probes_sent), RepliesSent: uint64(raw.clock_replies_sent),
			Samples: uint64(raw.clock_samples), Rejected: uint64(raw.clock_rejected),
			SendFailures: uint64(raw.clock_send_failures),
		},
		Logger: GameLoggerMetrics{
			Dropped: uint64(raw.logger_dropped), SinkPanics: uint64(raw.logger_sink_panics),
			CallbackSamples: uint64(raw.logger_callback_samples),
			CallbackP99:     microseconds(raw.logger_callback_p99_us), CallbackMax: microseconds(raw.logger_callback_max_us),
		},
	}
	if r.loggerSink != nil {
		// The C dispatcher cannot observe a panic recovered by the Go callback bridge.
		metrics.Logger.SinkPanics += r.loggerSink.panics.Load()
	}
	return metrics, nil
}

func (r *GameRuntime) RealtimeQueueSnapshot() (GameRealtimeQueue, error) {
	handle, err := r.handleValue()
	if err != nil {
		return GameRealtimeQueue{}, err
	}
	var raw C.rnet_game_realtime_queue_t
	if err := statusError(C.rnet_game_realtime_queue_snapshot(handle, &raw)); err != nil {
		return GameRealtimeQueue{}, err
	}
	return GameRealtimeQueue{
		QueuedMessages: uint64(raw.queued_messages), QueuedBytes: uint64(raw.queued_bytes),
		AdmissionRejected: uint64(raw.admission_rejected), Replaced: uint64(raw.replaced),
		ClosedDropped: uint64(raw.closed_dropped), BackpressureDropped: uint64(raw.backpressure_dropped),
		SendFailed: uint64(raw.send_failed), Forwarded: uint64(raw.forwarded),
	}, nil
}
