package rnet

/*
#include "native.h"
*/
import "C"

import "time"

// GameRealtimeQueue reports runtime-wide LatestOnly staging; Forwarded means
// admitted to the transport queue, not delivered to the peer.
type GameRealtimeQueue struct {
	// QueuedMessages current distinct pending (session,key) entries in game staging.
	QueuedMessages uint64
	// QueuedBytes current payload plus game-envelope reservations, not allocator RSS.
	QueuedBytes uint64
	// AdmissionRejected cumulative staging budget/key-cap rejections, including failed requeues.
	AdmissionRejected uint64
	// Replaced cumulative old pending snapshots superseded by newer same-key values.
	Replaced uint64
	// ClosedDropped cumulative staged values discarded during session cleanup/shutdown.
	ClosedDropped uint64
	// BackpressureDropped cumulative values unable to reenter staging after transport backpressure.
	BackpressureDropped uint64
	// SendFailed cumulative values discarded after non-backpressure forwarding failures.
	SendFailed uint64
	// Forwarded cumulative snapshots accepted by transport, not delivered to the peer.
	Forwarded uint64
}

// GameScheduledQueue combines current staging gauges with cumulative scheduler outcomes.
// Forwarded is local transport admission, not delivery; counters do not reset on read.
type GameScheduledQueue struct {
	// QueuedMessages current admitted messages awaiting forwarding or drop.
	QueuedMessages uint64
	// QueuedBytes current payload plus worst-case game-envelope reservations.
	QueuedBytes uint64
	// AdmissionRejected cumulative staging byte/message budget rejections.
	AdmissionRejected uint64
	// ExpiredDropped cumulative messages expiring locally before forwarding.
	ExpiredDropped uint64
	// ClosedDropped cumulative messages discarded on session cleanup/shutdown.
	ClosedDropped uint64
	// SendFailed cumulative non-backpressure transport forwarding failures.
	SendFailed uint64
	// Forwarded cumulative transport admissions, not deliveries.
	Forwarded uint64
	// BackpressureRequeued cumulative retries returned to staging; one message can be counted repeatedly.
	BackpressureRequeued uint64
	// AdmittedByPriority cumulative admissions indexed low, normal, high, critical.
	AdmittedByPriority [4]uint64
	// ForwardedByPriority cumulative transport forwards in low, normal, high, critical order.
	ForwardedByPriority [4]uint64
	// QueueDelaySamples cumulative successful-forward delay samples; admission to transport, not delivery.
	QueueDelaySamples uint64
	// QueueDelayP90 estimated cumulative local queue-delay P90 as a time.Duration (native microsecond precision).
	QueueDelayP90 time.Duration
	// QueueDelayP95 estimated cumulative local queue-delay P95 as a time.Duration (native microsecond precision).
	QueueDelayP95 time.Duration
	// QueueDelayP99 estimated cumulative local queue-delay P99 as a time.Duration (native microsecond precision).
	QueueDelayP99 time.Duration
	// QueueDelayMax largest sampled local queue delay as a time.Duration (native microsecond precision).
	QueueDelayMax time.Duration
	// TickQueueDelaySamples same delay sample count, restricted to messages carrying a tick.
	TickQueueDelaySamples uint64
	// TickQueueDelayP90 tick-bearing messages' estimated cumulative queue-delay P90, converted from native microseconds.
	TickQueueDelayP90 time.Duration
	// TickQueueDelayP95 tick-bearing messages' estimated cumulative queue-delay P95, converted from native microseconds.
	TickQueueDelayP95 time.Duration
	// TickQueueDelayP99 tick-bearing messages' estimated cumulative queue-delay P99, converted from native microseconds.
	TickQueueDelayP99 time.Duration
	// TickQueueDelayMax largest sampled tick-bearing message queue delay, converted from native microseconds.
	TickQueueDelayMax time.Duration
}

// GameRangeBuffer reports runtime-wide early-data gauges and cumulative negotiation-buffer rejections.
type GameRangeBuffer struct {
	// BufferedMessages current early messages retained, including ready-but-not-yet-polled messages.
	BufferedMessages uint64
	// BufferedBytes current early payload bytes; excludes framing/allocator overhead.
	BufferedBytes uint64
	// PeakBufferedMessages highest retained early-message count observed.
	PeakBufferedMessages uint64
	// PeakBufferedBytes highest retained early payload bytes observed.
	PeakBufferedBytes uint64
	// MaxBufferedMessages runtime-wide cap inherited from configured event queue capacity.
	MaxBufferedMessages uint64
	// MaxBufferedBytes runtime-wide cap inherited from configured event-byte budget.
	MaxBufferedBytes uint64
	// SessionAdmissionRejected cumulative per-session early-data budget rejections.
	SessionAdmissionRejected uint64
	// RuntimeAdmissionRejected cumulative runtime-wide early-data budget rejections.
	RuntimeAdmissionRejected uint64
}

// GameMetrics contains runtime-scoped cumulative game counters without player labels.
// LoggerAvailable distinguishes a disabled logger from a logger with zero drops.
type GameMetrics struct {
	// LoggerAvailable whether a logger is installed; distinguishes disabled logging from zero drops.
	LoggerAvailable bool
	// Heartbeat cumulative protected heartbeat counters and RTT samples.
	Heartbeat GameHeartbeatMetrics
	// Resume cumulative recovery outcomes plus a live retained-ticket gauge, scoped to this runtime.
	Resume GameResumeMetrics
	// Clock cumulative approximate clock-sampling counters.
	Clock GameClockMetrics
	// Logger cumulative bounded logger counters and callback timing; includes recovered Go sink panics.
	Logger GameLoggerMetrics
}

// GameHeartbeatMetrics counts probes, acknowledgements and heartbeat failure outcomes since creation.
type GameHeartbeatMetrics struct {
	// ProbesSent cumulative protected probes admitted locally, not confirmed delivered.
	ProbesSent uint64
	// ProbeSendFailures cumulative failed local probe sends.
	ProbeSendFailures uint64
	// RepliesSent cumulative protected replies admitted locally.
	RepliesSent uint64
	// ReplySendFailures cumulative failed local reply sends.
	ReplySendFailures uint64
	// RepliesMatched cumulative timely replies matching outstanding local challenges.
	RepliesMatched uint64
	// RepliesRejected cumulative invalid/late challenge replies; do not refresh liveness.
	RepliesRejected uint64
	// ProbesRateLimited cumulative inbound probes suppressed by response rate limits.
	ProbesRateLimited uint64
	// Timeouts cumulative heartbeat deadlines causing session-close requests.
	Timeouts uint64
	// RTTSamples cumulative accepted RTT samples; zero means unavailable.
	RTTSamples uint64
	// RTTP50 cumulative estimated heartbeat RTT median as a time.Duration (native microsecond precision).
	RTTP50 time.Duration
	// RTTP90 cumulative estimated heartbeat RTT P90 as a time.Duration (native microsecond precision).
	RTTP90 time.Duration
	// RTTP95 cumulative estimated heartbeat RTT P95 as a time.Duration (native microsecond precision).
	RTTP95 time.Duration
	// RTTP99 cumulative estimated heartbeat RTT P99 as a time.Duration (native microsecond precision).
	RTTP99 time.Duration
	// RTTP999 cumulative estimated heartbeat RTT P99.9 as a time.Duration (native microsecond precision).
	RTTP999 time.Duration
	// RTTMax maximum sampled heartbeat RTT as a time.Duration (native microsecond precision).
	RTTMax time.Duration
}

// GameResumeMetrics counts same-runtime one-use recovery ticket and takeover outcomes.
type GameResumeMetrics struct {
	// TicketsIssued cumulative tickets issued and admitted to protected send, not confirmed delivered.
	TicketsIssued uint64
	// RequestsReceived cumulative decoded resume joins reaching ticket validation.
	RequestsReceived uint64
	// TicketsRejected cumulative invalid, expired, replayed or incompatible ticket claims.
	TicketsRejected uint64
	// AuthorizationDenied cumulative valid resume claims denied by business authorization.
	AuthorizationDenied uint64
	// PendingRevoked cumulative pending/in-flight resume claims revoked by explicit cleanup.
	PendingRevoked uint64
	// SessionsResumed cumulative completed resume publications on this runtime.
	SessionsResumed uint64
	// OutstandingTickets current retained registry entries; expiry cleanup is opportunistic.
	OutstandingTickets uint64
}

// GameClockMetrics counts protected approximate clock-sampling outcomes, not trusted simulation time.
type GameClockMetrics struct {
	// ProbesSent cumulative locally admitted client clock probes.
	ProbesSent uint64
	// RepliesSent cumulative locally admitted server clock replies.
	RepliesSent uint64
	// Samples cumulative accepted client clock samples.
	Samples uint64
	// Rejected cumulative rate-limited probes or tracker-rejected replies, not every parser error.
	Rejected uint64
	// SendFailures cumulative probe generation/admission and reply send-admission failures.
	SendFailures uint64
}

// GameLoggerMetrics reports bounded logger outcomes; QueueDepth is current, other fields cumulative.
type GameLoggerMetrics struct {
	// Dropped cumulative records rejected by bounded logger queues.
	Dropped uint64
	// SinkPanics cumulative caught native sink panics plus Go callback panics recovered by the bridge.
	SinkPanics uint64
	// CallbackSamples completed callback samples, including caught panics.
	CallbackSamples uint64
	// CallbackP99 cumulative estimated callback execution P99 as a time.Duration (native microsecond precision); excludes queue wait.
	CallbackP99 time.Duration
	// CallbackMax largest measured callback execution time as a time.Duration (native microsecond precision).
	CallbackMax time.Duration
}

// MetricsSnapshot copies game counters without resetting them, including recovered Go logger panics.
// Fields may change concurrently and are not a single atomic snapshot.
func (r *GameRuntime) MetricsSnapshot() (GameMetrics, error) {
	defer lockNativeThread()()
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

// RealtimeQueueSnapshot copies snapshot-staging gauges and cumulative outcomes without resetting them.
func (r *GameRuntime) RealtimeQueueSnapshot() (GameRealtimeQueue, error) {
	defer lockNativeThread()()
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

// ScheduledQueueSnapshot copies priority-staging gauges and cumulative outcomes without resetting them.
// Tick queue latency covers only messages carrying a tick; inspect its sample count.
func (r *GameRuntime) ScheduledQueueSnapshot() (GameScheduledQueue, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return GameScheduledQueue{}, err
	}
	var raw C.rnet_game_scheduled_queue_t
	if err := statusError(C.rnet_game_scheduled_queue_snapshot(handle, &raw)); err != nil {
		return GameScheduledQueue{}, err
	}
	value := GameScheduledQueue{
		QueuedMessages: uint64(raw.queued_messages), QueuedBytes: uint64(raw.queued_bytes),
		AdmissionRejected: uint64(raw.admission_rejected), ExpiredDropped: uint64(raw.expired_dropped),
		ClosedDropped: uint64(raw.closed_dropped), SendFailed: uint64(raw.send_failed), Forwarded: uint64(raw.forwarded),
		BackpressureRequeued: uint64(raw.backpressure_requeued),
		QueueDelaySamples:    uint64(raw.queue_delay_samples),
		QueueDelayP90:        microseconds(raw.queue_delay_p90_us), QueueDelayP95: microseconds(raw.queue_delay_p95_us),
		QueueDelayP99: microseconds(raw.queue_delay_p99_us), QueueDelayMax: microseconds(raw.queue_delay_max_us),
		TickQueueDelaySamples: uint64(raw.tick_queue_delay_samples),
		TickQueueDelayP90:     microseconds(raw.tick_queue_delay_p90_us), TickQueueDelayP95: microseconds(raw.tick_queue_delay_p95_us),
		TickQueueDelayP99: microseconds(raw.tick_queue_delay_p99_us), TickQueueDelayMax: microseconds(raw.tick_queue_delay_max_us),
	}
	for index := 0; index < 4; index++ {
		value.AdmittedByPriority[index] = uint64(raw.admitted_by_priority[index])
		value.ForwardedByPriority[index] = uint64(raw.forwarded_by_priority[index])
	}
	return value, nil
}

// RangeBufferSnapshot returns current early-data capacity and cumulative rejection telemetry.
// It is runtime-wide and does not expose per-player labels.
func (r *GameRuntime) RangeBufferSnapshot() (GameRangeBuffer, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return GameRangeBuffer{}, err
	}
	var raw C.rnet_game_range_buffer_t
	if err := statusError(C.rnet_game_range_buffer_snapshot(handle, &raw)); err != nil {
		return GameRangeBuffer{}, err
	}
	return GameRangeBuffer{
		BufferedMessages: uint64(raw.buffered_messages), BufferedBytes: uint64(raw.buffered_bytes),
		PeakBufferedMessages: uint64(raw.peak_buffered_messages), PeakBufferedBytes: uint64(raw.peak_buffered_bytes),
		MaxBufferedMessages: uint64(raw.max_buffered_messages), MaxBufferedBytes: uint64(raw.max_buffered_bytes),
		SessionAdmissionRejected: uint64(raw.session_admission_rejected), RuntimeAdmissionRejected: uint64(raw.runtime_admission_rejected),
	}, nil
}
