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

// GameProtocol is an exact application protocol identity, checked before business authorization.
type GameProtocol struct {
	ID           uint64
	Version      uint32
	BuildID      uint64
	Capabilities uint64
}

type GameConfig struct {
	HeartbeatInterval          time.Duration
	HeartbeatTimeout           time.Duration
	AllowPlaintextBusinessData bool
	// Network optionally overrides production transport limits; start from DefaultConfig.
	Network *Config
	// Logger receives bounded, asynchronous game lifecycle/security/quality records.
	// It must return quickly and never call Stop or Close on its own runtime.
	Logger      func(LogRecord)
	MinLogLevel LogLevel
}

type GameServerConfig struct {
	Transport Transport
	Host      string
	Port      uint16
	Keypair   Keypair
	// Zero means encrypted; plaintext must be chosen explicitly.
	InitialSecurity SecurityMode
	Protocol        GameProtocol
}

func (config GameServerConfig) String() string {
	return fmt.Sprintf("GameServerConfig{Transport:%d Host:%q Port:%d InitialSecurity:%d ProtocolID:%d Version:%d Keypair:<redacted>}",
		config.Transport, config.Host, config.Port, config.InitialSecurity, config.Protocol.ID, config.Protocol.Version)
}

func (config GameServerConfig) GoString() string { return config.String() }

type GameClientConfig struct {
	Transport  Transport
	Host       string
	Port       uint16
	JoinTicket []byte
	Protocol   GameProtocol
}

func (config GameClientConfig) String() string {
	return fmt.Sprintf("GameClientConfig{Transport:%d Host:%q Port:%d ProtocolID:%d Version:%d JoinTicketLen:%d}",
		config.Transport, config.Host, config.Port, config.Protocol.ID, config.Protocol.Version, len(config.JoinTicket))
}

func (config GameClientConfig) GoString() string { return config.String() }

type GameEventType uint32

const (
	GameRuntimeStarted    GameEventType = C.RNET_GAME_RUNTIME_STARTED
	GameEndpointOpened    GameEventType = C.RNET_GAME_ENDPOINT_OPENED
	GameEndpointError     GameEventType = C.RNET_GAME_ENDPOINT_ERROR
	GameAuthRequest       GameEventType = C.RNET_GAME_AUTH_REQUEST
	GameResumeRequest     GameEventType = C.RNET_GAME_RESUME_REQUEST
	GameProtocolRejected  GameEventType = C.RNET_GAME_PROTOCOL_REJECTED
	GameSessionReady      GameEventType = C.RNET_GAME_SESSION_READY
	GameSessionResumed    GameEventType = C.RNET_GAME_SESSION_RESUMED
	GameResumeTicket      GameEventType = C.RNET_GAME_RESUME_TICKET
	GameSessionClosed     GameEventType = C.RNET_GAME_SESSION_CLOSED
	GameMessage           GameEventType = C.RNET_GAME_MESSAGE
	GameWritable          GameEventType = C.RNET_GAME_WRITABLE
	GameJoinFailed        GameEventType = C.RNET_GAME_JOIN_FAILED
	GameSecurityChanged   GameEventType = C.RNET_GAME_SECURITY_CHANGED
	GameQualityChanged    GameEventType = C.RNET_GAME_QUALITY_CHANGED
	GameProtocolViolation GameEventType = C.RNET_GAME_PROTOCOL_VIOLATION
	GameRuntimeStopped    GameEventType = C.RNET_GAME_RUNTIME_STOPPED
)

// GameEvent owns copies of its payloads. The caller is responsible for erasing
// copied join credentials and resume tickets when no longer needed.
type GameEvent struct {
	Type              GameEventType
	Endpoint          Endpoint
	Session           Session
	RelatedSession    Session
	Status            int32
	Data              []byte
	AuxData           []byte
	ClientPublicKey   [32]byte
	BuildID           uint64
	Capabilities      uint64
	Encrypted         bool
	SecurityEpoch     uint64
	SecurityOperation SecurityOperation
	QualityGrade      uint32
	QualityBasis      uint32
	LastRTT           time.Duration
	Jitter            time.Duration
	QualitySamples    uint64
	HasSequence       bool
	Sequence          uint32
	HasTick           bool
	Tick              uint32
}

// GameQuality distinguishes unavailable TCP loss from a measured zero.
// UDP loss is a sequence-gap estimate; KCP reports retransmissions, not IP loss.
type GameQuality struct {
	Available                       bool
	Grade                           uint32
	Basis                           uint32
	HasUDPLoss                      bool
	HasKCPRetransmissions           bool
	UDPRecentLossPerMille           uint32
	KCPRecentRetransmissionPerMille uint32
	LastRTT                         time.Duration
	SmoothedRTT                     time.Duration
	Jitter                          time.Duration
	Samples                         uint64
	UDPExpected                     uint64
	UDPMissing                      uint64
	KCPSegmentsSent                 uint64
	KCPRetransmitted                uint64
}

// GameClockSync maps this client's runtime-local monotonic clock to the
// server's runtime-local monotonic clock. It is not UTC or a trusted time source.
type GameClockSync struct {
	Available         bool
	ServerMinusClient time.Duration
	RTT               time.Duration
	Samples           uint64
}

func (event GameEvent) String() string {
	return fmt.Sprintf("GameEvent{Type:%d Endpoint:%d Session:%d RelatedSession:%d Status:%d DataLen:%d AuxDataLen:%d}",
		event.Type, event.Endpoint, event.Session, event.RelatedSession, event.Status,
		len(event.Data), len(event.AuxData))
}

func (event GameEvent) GoString() string { return event.String() }

type GameRuntime struct {
	mu         sync.RWMutex
	handle     C.rnet_runtime_t
	stopped    bool
	logger     cgo.Handle
	loggerSink *gameLoggerSink
}

// NewGameRuntime creates a production-default game runtime. Pass nil for a
// server-only runtime; clients pin one expected server public key.
func NewGameRuntime(security *ClientSecurity, configs ...GameConfig) (*GameRuntime, error) {
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
	}
	var privateKey, serverKey *C.uint8_t
	var network C.rnet_config_v5_t
	var networkPtr *C.rnet_config_v5_t
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

func (r *GameRuntime) Listen(config GameServerConfig) (Endpoint, error) {
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	var encrypted C.uint32_t = 1
	if config.InitialSecurity == SecurityPlaintext {
		encrypted = 0
	} else if config.InitialSecurity != 0 && config.InitialSecurity != SecurityEncrypted {
		return 0, fmt.Errorf("rnet: invalid game initial security mode")
	}
	status := C.rnet_go_game_server_listen(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), (*C.uint8_t)(unsafe.Pointer(&config.Keypair.Private[0])),
		encrypted, C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.Version), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	return Endpoint(endpoint), statusError(status)
}

func (r *GameRuntime) Connect(config GameClientConfig) (Endpoint, error) {
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_client_connect(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinTicket), C.size_t(len(config.JoinTicket)),
		C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.Version),
		C.uint64_t(config.Protocol.BuildID), C.uint64_t(config.Protocol.Capabilities), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	return Endpoint(endpoint), statusError(status)
}

// ConnectResume performs a new handshake; a server ResumeRequest still requires AuthDecide.
func (r *GameRuntime) ConnectResume(config GameClientConfig, oldSession Session, ticket []byte) (Endpoint, error) {
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_client_resume_connect(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinTicket), C.size_t(len(config.JoinTicket)),
		C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.Version),
		C.uint64_t(config.Protocol.BuildID), C.uint64_t(config.Protocol.Capabilities),
		C.rnet_session_t(oldSession), bytePointer(ticket), C.size_t(len(ticket)), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	runtime.KeepAlive(ticket)
	return Endpoint(endpoint), statusError(status)
}

// IssueResumeTicket sends a one-use, server-authoritative ticket to the ready client.
// Identity is opaque to RNet and must not be logged.
func (r *GameRuntime) IssueResumeTicket(session Session, identity []byte) error {
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(identity), len: C.size_t(len(identity))}
	status := C.rnet_game_issue_resume_ticket(handle, C.rnet_session_t(session), native)
	runtime.KeepAlive(identity)
	return statusError(status)
}

func (r *GameRuntime) LocalPort(endpoint Endpoint) (uint16, error) {
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	var port C.uint16_t
	err = statusError(C.rnet_game_endpoint_local_port(handle, C.rnet_endpoint_t(endpoint), &port))
	return uint16(port), err
}

func (r *GameRuntime) AuthDecide(session Session, accept bool) error {
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

func (r *GameRuntime) Send(session Session, payload []byte) error {
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(payload), len: C.size_t(len(payload))}
	status := C.rnet_game_send(handle, C.rnet_session_t(session), native)
	runtime.KeepAlive(payload)
	return statusError(status)
}

// SendLatest coalesces snapshots by session and key. TCP/KCP snapshots can also
// be replaced while waiting in a transport-pending slot; worker-owned data
// already handed to a socket or KCP cannot be withdrawn.
func (r *GameRuntime) SendLatest(session Session, key uint64, payload []byte) error {
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	native := C.rnet_slice_t{ptr: bytePointer(payload), len: C.size_t(len(payload))}
	status := C.rnet_game_send_latest(handle, C.rnet_session_t(session), C.uint64_t(key), native)
	runtime.KeepAlive(payload)
	return statusError(status)
}

func (r *GameRuntime) CloseSession(session Session) error {
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_game_session_close(handle, C.rnet_session_t(session)))
}

func (r *GameRuntime) CloseEndpoint(endpoint Endpoint) error {
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_game_endpoint_close(handle, C.rnet_endpoint_t(endpoint)))
}

func (r *GameRuntime) Rekey(session Session) error {
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_game_rekey(handle, C.rnet_session_t(session)))
}

func (r *GameRuntime) SetEncryption(session Session, enabled bool) error {
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

func (r *GameRuntime) NetworkQuality(session Session) (GameQuality, error) {
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

// ClockMicros returns microseconds since this runtime's monotonic origin.
func (r *GameRuntime) ClockMicros() (uint64, error) {
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

// ClockSyncSnapshot returns the latest client-side four-timestamp sample.
func (r *GameRuntime) ClockSyncSnapshot(session Session) (GameClockSync, error) {
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

// PrometheusSnapshot returns an owned text copy without per-player labels.
func (r *GameRuntime) PrometheusSnapshot() (string, error) {
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

func (r *GameRuntime) Poll(capacity int, timeout time.Duration) (result []GameEvent, err error) {
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
	if err = statusError(C.rnet_game_poll_events(handle, pointer, C.size_t(capacity), C.uint32_t(timeoutMS), &count)); err != nil {
		return nil, err
	}
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

func (r *GameRuntime) Stop(timeout time.Duration) error {
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

func (r *GameRuntime) Close() error {
	if err := r.Stop(0); err != nil {
		return err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.handle == 0 {
		return nil
	}
	if err := statusError(C.rnet_game_runtime_destroy(r.handle)); err != nil {
		return err
	}
	r.handle = 0
	if r.logger != 0 {
		r.logger.Delete()
		r.logger = 0
	}
	return nil
}
