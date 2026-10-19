package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"math"
	"runtime"
	"runtime/cgo"
	"sync"
	"time"
	"unsafe"
)

// GameRuntime owns native game state and workers for both server and client endpoints.
// Do not copy it after use; Close explicitly. Continuously Poll to advance negotiation,
// heartbeats, clock samples and staging queues. Use one event consumer; stop concurrent
// callers before shutdown. Logger callbacks must never call Stop or Close.
type GameRuntime struct {
	mu         sync.RWMutex
	handle     C.rnet_runtime_t
	stopped    bool
	closing    bool
	logger     cgo.Handle
	loggerSink *gameLoggerSink
}

// NewGameRuntime starts workers using zero or one GameConfig; its zero value selects defaults.
// security nil creates a server-only runtime; outbound clients need a local key and
// one pinned server public key. Config/key bytes are copied, but Logger is retained.
// Invalid configuration, extra configs or an incompatible native ABI returns an error.
func NewGameRuntime(security *ClientSecurity, configs ...GameConfig) (*GameRuntime, error) {
	if err := checkABI(); err != nil {
		return nil, err
	}
	defer lockNativeThread()()
	if len(configs) > 1 {
		return nil, fmt.Errorf("rnet: NewGameRuntime accepts at most one config")
	}
	var native C.rnet_game_config_t
	if err := statusError(C.rnet_game_config_init(&native)); err != nil {
		return nil, err
	}
	if len(configs) == 1 {
		config := configs[0]
		if config.HeartbeatInterval < 0 || config.HeartbeatTimeout < 0 ||
			config.HeartbeatInterval.Milliseconds() > math.MaxUint32 ||
			config.HeartbeatTimeout.Milliseconds() > math.MaxUint32 {
			return nil, fmt.Errorf("rnet: invalid game heartbeat duration")
		}
		native.heartbeat_interval_ms = C.uint32_t(config.HeartbeatInterval.Milliseconds())
		native.heartbeat_timeout_ms = C.uint32_t(config.HeartbeatTimeout.Milliseconds())
		if config.AllowPlaintextBusinessData {
			native.allow_plaintext_business_data = 1
		}
		native.realtime_max_queued_bytes = C.uint64_t(config.RealtimeQueue.MaxQueuedBytes)
		native.realtime_max_session_queued_bytes = C.uint64_t(config.RealtimeQueue.MaxSessionQueuedBytes)
		native.realtime_max_keys_per_session = C.uint64_t(config.RealtimeQueue.MaxKeysPerSession)
		native.realtime_flush_batch = C.uint64_t(config.RealtimeQueue.FlushBatch)
		native.scheduled_max_queued_bytes = C.uint64_t(config.ScheduledQueue.MaxQueuedBytes)
		native.scheduled_max_session_queued_bytes = C.uint64_t(config.ScheduledQueue.MaxSessionQueuedBytes)
		native.scheduled_max_queued_messages = C.uint64_t(config.ScheduledQueue.MaxQueuedMessages)
		native.scheduled_flush_batch = C.uint64_t(config.ScheduledQueue.FlushBatch)
	}
	var privateKey, serverKey *C.uint8_t
	var network C.rnet_config_t
	var networkPtr *C.rnet_config_t
	if len(configs) == 1 && configs[0].Network != nil {
		configured, networkErr := gameNetworkConfig(*configs[0].Network)
		if networkErr != nil {
			return nil, networkErr
		}
		network = configured
		networkPtr = &network
	}
	if security != nil {
		if len(security.ExpectedServerPublicKey) != 32 {
			return nil, fmt.Errorf("rnet: expected server public key must contain 32 bytes")
		}
		privateKey = (*C.uint8_t)(unsafe.Pointer(&security.LocalKey.Private[0]))
		serverKey = bytePointer(security.ExpectedServerPublicKey)
	}
	var handle C.rnet_runtime_t
	var loggerHandle cgo.Handle
	var loggerSink *gameLoggerSink
	if len(configs) == 1 && configs[0].Logger != nil {
		loggerSink = &gameLoggerSink{callback: configs[0].Logger}
		loggerHandle = cgo.NewHandle(loggerSink)
	}
	var minLogLevel LogLevel
	if len(configs) == 1 {
		minLogLevel = configs[0].MinLogLevel
	}
	status := C.rnet_go_game_runtime_create(&native, networkPtr, privateKey, serverKey,
		C.uintptr_t(loggerHandle), C.uint32_t(minLogLevel), &handle)
	runtime.KeepAlive(security)
	if err := statusError(status); err != nil {
		if loggerHandle != 0 {
			loggerHandle.Delete()
		}
		return nil, err
	}
	return &GameRuntime{handle: handle, logger: loggerHandle, loggerSink: loggerSink}, nil
}

func (r *GameRuntime) handleValue() (C.rnet_runtime_t, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	if r.handle == 0 {
		return 0, StatusError{Code: int32(C.RNET_E_INVALID_HANDLE), Message: "game runtime is closed"}
	}
	return r.handle, nil
}

// IssueResumeTicket queues a one-use credential for a ready server-side session.
// identity must contain 1..64 opaque bytes; the client receives GameResumeTicket.
// Tickets require this same server runtime and reauthorization, not restart/cluster recovery.
// The identity is copied; never log it or the resulting ticket.
func (r *GameRuntime) IssueResumeTicket(session Session, identity []byte) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(identity), len: C.size_t(len(identity))}
	status := C.rnet_game_issue_resume_ticket(handle, C.rnet_session_t(session), native)
	runtime.KeepAlive(identity)
	return statusError(status)
}

// LocalPort returns a live endpoint's bound port, including a port assigned by Listen.
// The endpoint must belong to this runtime; stale handles return an error.
func (r *GameRuntime) LocalPort(endpoint Endpoint) (uint16, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	var port C.uint16_t
	err = statusError(C.rnet_game_endpoint_local_port(handle, C.rnet_endpoint_t(endpoint), &port))
	return uint16(port), err
}

// AuthDecide accepts or rejects a pending GameAuthRequest or GameResumeRequest.
// Successful acceptance only queues authorization; await GameSessionReady or
// GameSessionResumed before sending. A pending resume must be authorized again.
func (r *GameRuntime) AuthDecide(session Session, accept bool) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	var decision C.uint32_t
	if accept {
		decision = 1
	}
	return statusError(C.rnet_game_auth_decide(handle, C.rnet_session_t(session), decision))
}

// Send copies payload into the normal-priority game staging queue for a ready session.
// Nil is a valid empty message. Success is local admission, not delivery; backpressure,
// size limits and non-ready/closed sessions return errors. Poll forwards staged traffic.
func (r *GameRuntime) Send(session Session, payload []byte) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(payload), len: C.size_t(len(payload))}
	status := C.rnet_game_send(handle, C.rnet_session_t(session), native)
	runtime.KeepAlive(payload)
	return statusError(status)
}

// SendWithOptions follows Send's ownership/admission contract with optional metadata,
// priority and expiry. Zero priority selects normal; negative expiry is rejected.
// Expiry only affects game staging and cannot retract transport-admitted data.
func (r *GameRuntime) SendWithOptions(session Session, payload []byte, options GameSendOptions) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	priority := options.Priority
	if priority == 0 {
		priority = GamePriorityNormal
	}
	expiryMS, err := gameExpiryMilliseconds(options.Expiry)
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(payload), len: C.size_t(len(payload))}
	raw := C.rnet_game_send_options_t{
		struct_size: C.uint32_t(C.sizeof_rnet_game_send_options_t),
		abi_version: C.RNET_ABI_VERSION,
		sequence:    C.uint32_t(options.Sequence), tick: C.uint32_t(options.Tick),
		correlation_id: C.uint64_t(options.CorrelationID), priority: C.uint32_t(priority),
		expiry_ms: C.uint64_t(expiryMS),
	}
	if options.HasSequence {
		raw.has_sequence = 1
	}
	if options.HasTick {
		raw.has_tick = 1
	}
	status := C.rnet_game_send_ex(handle, C.rnet_session_t(session), native, &raw)
	runtime.KeepAlive(payload)
	return statusError(status)
}

func gameExpiryMilliseconds(expiry time.Duration) (uint64, error) {
	if expiry < 0 {
		return 0, fmt.Errorf("rnet: game send expiry must not be negative")
	}
	milliseconds := expiry / time.Millisecond
	if expiry%time.Millisecond != 0 {
		milliseconds++
	}
	return uint64(milliseconds), nil
}

// SendLatest copies a replaceable snapshot under (session, key); key zero is valid.
// A ready session is required. All transports replace pending game snapshots; TCP/KCP
// also replace transport-pending slots before worker pickup. Worker-owned socket/KCP
// data cannot be withdrawn. Success is admission, not delivery; Poll forwards snapshots.
func (r *GameRuntime) SendLatest(session Session, key uint64, payload []byte) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(payload), len: C.size_t(len(payload))}
	status := C.rnet_game_send_latest(handle, C.rnet_session_t(session), C.uint64_t(key), native)
	runtime.KeepAlive(payload)
	return statusError(status)
}

// CloseSession closes the local session, clears staging and revokes its resume state.
// It is an explicit kick, not a recoverable network disconnect; stale handles fail.
func (r *GameRuntime) CloseSession(session Session) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_game_session_close(handle, C.rnet_session_t(session)))
}

// CloseEndpoint closes a local listener/connection endpoint and its owned sessions.
// Do not send using their old handles after closure.
func (r *GameRuntime) CloseEndpoint(endpoint Endpoint) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_game_endpoint_close(handle, C.rnet_endpoint_t(endpoint)))
}

// Rekey queues a server-led key rotation for a ready session without changing its mode.
// GameSecurityChanged reports local completion; client-side or invalid states fail.
func (r *GameRuntime) Rekey(session Session) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_game_rekey(handle, C.rnet_session_t(session)))
}

// SetEncryption requests a ready server session's business encryption policy.
// Clients follow automatically. Disabling requires AllowPlaintextBusinessData and
// removes business confidentiality/integrity; control traffic stays protected.
// Await GameSecurityChanged unless the requested mode was already active (a no-op).
func (r *GameRuntime) SetEncryption(session Session, enabled bool) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	var value C.uint32_t
	if enabled {
		value = 1
	}
	return statusError(C.rnet_game_security_set(handle, C.rnet_session_t(session), value))
}

// NetworkQuality returns evidence for a tracked session; success does not prove game readiness.
// Check Available and transport availability flags: TCP has no loss estimate,
// UDP measures sequence gaps, and KCP measures retransmissions rather than IP loss.
func (r *GameRuntime) NetworkQuality(session Session) (GameQuality, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return GameQuality{}, err
	}
	var raw C.rnet_game_quality_t
	if err := statusError(C.rnet_game_network_quality(handle, C.rnet_session_t(session), &raw)); err != nil {
		return GameQuality{}, err
	}
	return GameQuality{
		Available: raw.available != 0, Grade: uint32(raw.grade), Basis: uint32(raw.basis),
		HasUDPLoss: raw.has_udp_loss != 0, HasKCPRetransmissions: raw.has_kcp_retransmissions != 0,
		UDPRecentLossPerMille:           uint32(raw.udp_recent_loss_per_mille),
		KCPRecentRetransmissionPerMille: uint32(raw.kcp_recent_retransmission_per_mille),
		LastRTT:                         microseconds(C.uint64_t(raw.last_rtt_us)),
		SmoothedRTT:                     microseconds(C.uint64_t(raw.smoothed_rtt_us)),
		Jitter:                          microseconds(C.uint64_t(raw.jitter_us)), Samples: uint64(raw.samples),
		UDPExpected: uint64(raw.udp_expected), UDPMissing: uint64(raw.udp_missing),
		KCPSegmentsSent: uint64(raw.kcp_segments_sent), KCPRetransmitted: uint64(raw.kcp_retransmitted),
	}, nil
}

// ClockMicros returns microseconds since this runtime's monotonic origin, not UTC.
// Values from different runtimes require an offset estimate before comparison.
func (r *GameRuntime) ClockMicros() (uint64, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	var value C.uint64_t
	if err := statusError(C.rnet_game_clock_micros(handle, &value)); err != nil {
		return 0, err
	}
	return uint64(value), nil
}

// ClockSyncSnapshot returns the latest approximate client-side four-timestamp sample.
// Available is false on server sessions or before a sample. Queueing affects the estimate;
// it is not a trusted anti-cheat clock. Unknown sessions return an error.
func (r *GameRuntime) ClockSyncSnapshot(session Session) (GameClockSync, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return GameClockSync{}, err
	}
	var raw C.rnet_game_clock_sync_t
	if err := statusError(C.rnet_game_clock_sync_snapshot(handle, C.rnet_session_t(session), &raw)); err != nil {
		return GameClockSync{}, err
	}
	return GameClockSync{
		Available:         raw.available != 0,
		ServerMinusClient: time.Duration(raw.server_minus_client_us) * time.Microsecond,
		RTT:               microseconds(C.uint64_t(raw.rtt_us)),
		Samples:           uint64(raw.samples),
	}, nil
}

// PrometheusSnapshot returns owned runtime-scoped text without per-player labels.
// It includes recovered Go logger panics; concurrent updates are not an atomic snapshot.
func (r *GameRuntime) PrometheusSnapshot() (string, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return "", err
	}
	var raw C.rnet_game_buffer_t
	if err := statusError(C.rnet_game_prometheus_snapshot(handle, &raw)); err != nil {
		return "", err
	}
	if raw.token != 0 {
		defer C.rnet_game_buffer_release(handle, raw.token)
	}
	bytes, err := gameCopyBytes(raw.data, raw.len)
	if err != nil {
		return "", err
	}
	result := string(bytes)
	if r.loggerSink != nil {
		result += fmt.Sprintf("# TYPE rnet_game_go_logger_panics_total counter\nrnet_game_go_logger_panics_total %d\n", r.loggerSink.panics.Load())
	}
	return result, nil
}

// Poll consumes up to capacity game events and copies both payloads into Go-owned memory.
// capacity must be 0..65536; zero still advances internal maintenance. Call continually
// on both peers, even when idle. timeout is an idle wait, truncated to milliseconds and
// clamped to [0, MaxUint32], not a hard call deadline. Erase copied credentials promptly.
// Staged count/byte limits pause transport draining, not timers. Use positive capacity
// regularly; zero-only polling can block queued ACKs and cause heartbeat timeouts.
// On export failure, consumed events are not replayed; exported native buffers are released.
func (r *GameRuntime) Poll(capacity int, timeout time.Duration) (result []GameEvent, err error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return nil, err
	}
	if capacity < 0 {
		return nil, fmt.Errorf("rnet: game poll capacity must not be negative")
	}
	if capacity > 65536 {
		return nil, fmt.Errorf("rnet: game poll capacity is too large")
	}
	native := make([]C.rnet_game_event_t, capacity)
	var pointer *C.rnet_game_event_t
	if capacity > 0 {
		pointer = &native[0]
	}
	timeoutMS := timeout.Milliseconds()
	if timeoutMS < 0 {
		timeoutMS = 0
	} else if timeoutMS > math.MaxUint32 {
		timeoutMS = math.MaxUint32
	}
	var count C.size_t
	// A failed export may still own a prefix of buffers. Capture TLS diagnostics
	// before cleanup calls overwrite them, and register cleanup before returning.
	err = statusError(C.rnet_game_poll_events(handle, pointer, C.size_t(capacity), C.uint32_t(timeoutMS), &count))
	defer func() {
		for i := 0; i < int(count); i++ {
			for _, token := range []C.uint64_t{native[i].buffer_token, native[i].aux_buffer_token} {
				if token != 0 {
					if releaseErr := statusError(C.rnet_game_buffer_release(handle, token)); err == nil {
						err = releaseErr
					}
				}
			}
		}
	}()
	if err != nil {
		return nil, err
	}
	result = make([]GameEvent, 0, count)
	for i := 0; i < int(count); i++ {
		source := native[i]
		data, copyErr := gameCopyBytes(source.data, source.data_len)
		if copyErr != nil {
			return nil, copyErr
		}
		aux, copyErr := gameCopyBytes(source.aux_data, source.aux_data_len)
		if copyErr != nil {
			return nil, copyErr
		}
		event := GameEvent{
			Type: GameEventType(source.event_type), Endpoint: Endpoint(source.endpoint),
			Session: Session(source.session), RelatedSession: Session(source.related_session),
			Status: int32(source.status), Data: data, AuxData: aux,
			BuildID: uint64(source.build_id), Capabilities: uint64(source.capabilities),
			Encrypted: source.encrypted != 0, SecurityEpoch: uint64(source.security_epoch),
			SecurityOperation: SecurityOperation(source.security_operation),
			QualityGrade:      uint32(source.quality_grade), QualityBasis: uint32(source.quality_basis),
			LastRTT:        microseconds(C.uint64_t(source.last_rtt_us)),
			Jitter:         microseconds(C.uint64_t(source.jitter_us)),
			QualitySamples: uint64(source.quality_samples),
			HasSequence:    source.has_sequence != 0, Sequence: uint32(source.sequence),
			HasTick: source.has_tick != 0, Tick: uint32(source.tick),
			CorrelationID: uint64(source.correlation_id),
		}
		copy(event.ClientPublicKey[:], unsafe.Slice((*byte)(unsafe.Pointer(&source.client_public_key[0])), 32))
		result = append(result, event)
	}
	return result, nil
}

func gameCopyBytes(pointer *C.uint8_t, length C.size_t) ([]byte, error) {
	if length == 0 {
		return nil, nil
	}
	if pointer == nil || uint64(length) > math.MaxInt32 {
		return nil, fmt.Errorf("rnet: invalid game event buffer")
	}
	return C.GoBytes(unsafe.Pointer(pointer), C.int(length)), nil
}

// Stop requests native shutdown without destroying this wrapper. Already stopped/closed is a no-op.
// timeout is truncated to milliseconds and clamped to [0, MaxUint32], not a hard deadline.
// On backpressure, drain Poll and retry. Never call from this runtime's logger callback.
func (r *GameRuntime) Stop(timeout time.Duration) error {
	defer lockNativeThread()()
	r.mu.RLock()
	handle, stopped := r.handle, r.stopped
	r.mu.RUnlock()
	if handle == 0 || stopped {
		return nil
	}
	milliseconds := timeout.Milliseconds()
	if milliseconds < 0 {
		milliseconds = 0
	} else if milliseconds > math.MaxUint32 {
		milliseconds = math.MaxUint32
	}
	if err := statusError(C.rnet_game_runtime_stop(handle, C.uint32_t(milliseconds))); err != nil {
		return err
	}
	r.mu.Lock()
	r.stopped = true
	r.mu.Unlock()
	return nil
}

// Close stops and destroys the runtime and releases the retained logger after success.
// It is idempotent after success; failure preserves a handle for retry. Quiesce other
// callers first; concurrent Close can return backpressure. Never call from its logger.
func (r *GameRuntime) Close() error {
	defer lockNativeThread()()
	if err := r.Stop(0); err != nil {
		return err
	}
	r.mu.Lock()
	if r.closing {
		r.mu.Unlock()
		return StatusError{Code: int32(C.RNET_E_WOULD_BLOCK), Message: "game runtime is closing"}
	}
	if r.handle == 0 {
		r.mu.Unlock()
		return nil
	}
	handle := r.handle
	r.handle, r.closing = 0, true
	r.mu.Unlock()
	// Native destruction joins the logger. A pending callback must be able to query
	// this wrapper and receive a closed-handle error instead of waiting on our mutex.
	err := statusError(C.rnet_game_runtime_destroy(handle))
	r.mu.Lock()
	defer r.mu.Unlock()
	r.closing = false
	if err != nil {
		r.handle = handle
		return err
	}
	if r.logger != 0 {
		r.logger.Delete()
		r.logger = 0
	}
	return nil
}
