package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"time"
)

// GameProtocol is an exact application protocol identity, checked before business authorization.
type GameProtocol struct {
	ID           uint64
	Version      uint32
	BuildID      uint64
	Capabilities uint64
}

type GamePriority uint32

const (
	GamePriorityLow      GamePriority = C.RNET_GAME_PRIORITY_LOW
	GamePriorityNormal   GamePriority = C.RNET_GAME_PRIORITY_NORMAL
	GamePriorityHigh     GamePriority = C.RNET_GAME_PRIORITY_HIGH
	GamePriorityCritical GamePriority = C.RNET_GAME_PRIORITY_CRITICAL
)

// GameSendOptions carries optional network metadata. Business message typing stays in Payload.
type GameSendOptions struct {
	HasSequence   bool
	Sequence      uint32
	HasTick       bool
	Tick          uint32
	CorrelationID uint64
	Priority      GamePriority
	Expiry        time.Duration
}

type GameConfig struct {
	HeartbeatInterval          time.Duration
	HeartbeatTimeout           time.Duration
	AllowPlaintextBusinessData bool
	// Network optionally overrides production transport limits; start from DefaultConfig.
	Network *Config
	// Zero fields retain production defaults.
	RealtimeQueue  GameRealtimeQueueConfig
	ScheduledQueue GameScheduledQueueConfig
	// Logger receives bounded, asynchronous game lifecycle/security/quality records.
	// It must return quickly and never call Stop or Close on its own runtime.
	Logger      func(LogRecord)
	MinLogLevel LogLevel
}

type GameRealtimeQueueConfig struct {
	MaxQueuedBytes        uint64
	MaxSessionQueuedBytes uint64
	MaxKeysPerSession     uint64
	FlushBatch            uint64
}

type GameScheduledQueueConfig struct {
	MaxQueuedBytes        uint64
	MaxSessionQueuedBytes uint64
	MaxQueuedMessages     uint64
	FlushBatch            uint64
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
	CorrelationID     uint64
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
