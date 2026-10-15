package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"time"
)

// GamePriority selects weighted local staging service, not network QoS or delivery order guarantees.
type GamePriority uint32

const (
	// GamePriorityLow — Lowest local scheduler priority; no effect on remote dispatch.
	GamePriorityLow GamePriority = C.RNET_GAME_PRIORITY_LOW
	// GamePriorityNormal — Default priority used by ordinary game send.
	GamePriorityNormal GamePriority = C.RNET_GAME_PRIORITY_NORMAL
	// GamePriorityHigh — Higher local scheduler share, not a guaranteed deadline.
	GamePriorityHigh GamePriority = C.RNET_GAME_PRIORITY_HIGH
	// GamePriorityCritical — Highest local scheduler share; lower queues still receive service.
	GamePriorityCritical GamePriority = C.RNET_GAME_PRIORITY_CRITICAL
)

// GameSendOptions carries optional network metadata. Business message typing stays in Payload.
type GameSendOptions struct {
	// HasSequence whether to include Sequence; no deduplication or ordering is enabled.
	HasSequence bool
	// Sequence caller metadata copied unchanged when HasSequence is true; zero is valid.
	Sequence uint32
	// HasTick whether to include Tick.
	HasTick bool
	// Tick caller simulation tick when HasTick is true; not trusted time.
	Tick uint32
	// CorrelationID optional request/trace identifier copied to the peer; zero absent.
	CorrelationID uint64
	// Priority local weighted scheduler priority; zero means GamePriorityNormal in this Go wrapper.
	Priority GamePriority
	// Expiry relative game-staging lifetime; zero means no expiry, negative rejected, positive values round UP to milliseconds. Cannot retract transport-admitted bytes.
	Expiry time.Duration
}

// GameConfig is copied at construction; its zero value selects default game limits.
// It cannot reconfigure a live runtime. Use explicit runtime operations for security/logging changes.
type GameConfig struct {
	// HeartbeatInterval probe interval, truncated to whole milliseconds. Both resulting heartbeat values zero select 5s/15s defaults; negative or uint32-ms overflow rejected.
	HeartbeatInterval time.Duration
	// HeartbeatTimeout ack timeout, truncated to milliseconds. If either heartbeat value is nonzero, both must be positive and timeout at least interval.
	HeartbeatTimeout time.Duration
	// AllowPlaintextBusinessData permits local server plaintext policy, overriding Network's flag; outbound clients follow the server automatically.
	AllowPlaintextBusinessData bool
	// Network optional transport limits, copied at creation; seed with DefaultConfig. Its Logger and ClientSecurity must be nil.
	Network *Config
	// RealtimeQueue snapshot staging limits; each zero field keeps its native default.
	RealtimeQueue GameRealtimeQueueConfig
	// ScheduledQueue ordinary/priority staging limits; each zero field keeps its native default.
	ScheduledQueue GameScheduledQueueConfig
	// Logger optional serial asynchronous sink on this runtime's logging thread. Return promptly; never call Stop/Close. Panics are recovered and counted.
	Logger func(LogRecord)
	// MinLogLevel minimum inclusive severity; zero is LogTrace. Values above LogError clamp to LogError.
	MinLogLevel LogLevel
}

// GameRealtimeQueueConfig bounds replaceable snapshot staging separately from ordinary sends.
// Zero fields retain defaults; accepted snapshots still face transport backpressure.
type GameRealtimeQueueConfig struct {
	// MaxQueuedBytes runtime-wide snapshot staging byte budget; zero selects 16 MiB.
	MaxQueuedBytes uint64
	// MaxSessionQueuedBytes per-session snapshot staging byte budget; zero selects 256 KiB, cannot exceed runtime budget.
	MaxSessionQueuedBytes uint64
	// MaxKeysPerSession distinct pending keys per session; zero selects 64.
	MaxKeysPerSession uint64
	// FlushBatch snapshot forwarding attempts per Poll; zero selects 256.
	FlushBatch uint64
}

// GameScheduledQueueConfig bounds normal/priority game staging before transport admission.
// Zero fields retain defaults; expiration and fairness apply only while staged.
type GameScheduledQueueConfig struct {
	// MaxQueuedBytes runtime-wide ordinary/priority staging bytes; zero selects 16 MiB.
	MaxQueuedBytes uint64
	// MaxSessionQueuedBytes per-session ordinary/priority staging bytes; zero selects 256 KiB, cannot exceed total.
	MaxSessionQueuedBytes uint64
	// MaxQueuedMessages runtime staging message-count cap; zero selects 65536.
	MaxQueuedMessages uint64
	// FlushBatch ordinary/priority forwarding attempts per Poll; zero selects 256.
	FlushBatch uint64
}

// GameEventType identifies the valid fields of a GameEvent; do not infer kind from Status.
type GameEventType uint32

const (
	// GameRuntimeStarted — Game runtime started; no peer readiness implied.
	GameRuntimeStarted GameEventType = C.RNET_GAME_RUNTIME_STARTED
	// GameEndpointOpened — Endpoint socket opened; continue polling for authorization/readiness.
	GameEndpointOpened GameEventType = C.RNET_GAME_ENDPOINT_OPENED
	// GameEndpointError — Endpoint-related failure; query validity rather than assuming terminal closure.
	GameEndpointError GameEventType = C.RNET_GAME_ENDPOINT_ERROR
	// GameAuthRequest — Server authorization: data=login ticket, client key/build/capabilities identify request.
	GameAuthRequest GameEventType = C.RNET_GAME_AUTH_REQUEST
	// GameResumeRequest — Server reauthorization: data=identity, aux_data=fresh login, related_session=old handle.
	GameResumeRequest GameEventType = C.RNET_GAME_RESUME_REQUEST
	// GameProtocolRejected — Business protocol/range mismatch before application authorization.
	GameProtocolRejected GameEventType = C.RNET_GAME_PROTOCOL_REJECTED
	// GameSessionReady — Initial game join completed; session can send business data.
	GameSessionReady GameEventType = C.RNET_GAME_SESSION_READY
	// GameSessionResumed — Resume committed; session=new handle, related_session=old invalidated handle.
	GameSessionResumed GameEventType = C.RNET_GAME_SESSION_RESUMED
	// GameResumeTicket — Client receives sensitive single-runtime, single-use resume credential in data.
	GameResumeTicket GameEventType = C.RNET_GAME_RESUME_TICKET
	// GameSessionClosed — Session ended; status holds local close category.
	GameSessionClosed GameEventType = C.RNET_GAME_SESSION_CLOSED
	// GameMessage — Opaque business data plus optional sequence/tick/correlation metadata.
	GameMessage GameEventType = C.RNET_GAME_MESSAGE
	// GameWritable — Advisory send-capacity recovery; not a reservation or delivery receipt.
	GameWritable GameEventType = C.RNET_GAME_WRITABLE
	// GameJoinFailed — Join/negotiation failed; status describes local category.
	GameJoinFailed GameEventType = C.RNET_GAME_JOIN_FAILED
	// GameSecurityChanged — Local mode/key barrier completed; inspect encrypted/epoch/operation.
	GameSecurityChanged GameEventType = C.RNET_GAME_SECURITY_CHANGED
	// GameQualityChanged — Quality classification changed; inspect grade/basis/RTT/jitter/samples.
	GameQualityChanged GameEventType = C.RNET_GAME_QUALITY_CHANGED
	// GameProtocolViolation — Game/control protocol rejected; never a business message.
	GameProtocolViolation GameEventType = C.RNET_GAME_PROTOCOL_VIOLATION
	// GameRuntimeStopped — Game runtime stop completed; release buffers then destroy.
	GameRuntimeStopped GameEventType = C.RNET_GAME_RUNTIME_STOPPED
)

// GameEvent owns copies of its payloads. The caller is responsible for erasing
// copied join credentials and resume tickets when no longer needed.
type GameEvent struct {
	// Type game event discriminant; only kind-specific fields are meaningful.
	Type GameEventType
	// Endpoint local endpoint; zero for runtime-wide events.
	Endpoint Endpoint
	// Session current/pending local session, including NEW handle on GameSessionResumed; zero when not applicable.
	Session Session
	// RelatedSession old local handle on GameResumeRequest/GameSessionResumed; zero otherwise.
	RelatedSession Session
	// Status native error codes on error/rejection/close events; other kinds (including PROTOCOL_VIOLATION) use zero.
	Status int32
	// Data owned login ticket (GameAuthRequest), identity (GameResumeRequest), resume credential (GameResumeTicket), or business bytes (GameMessage).
	Data []byte
	// AuxData owned fresh login ticket on GameResumeRequest; empty otherwise.
	AuxData []byte
	// ClientPublicKey authenticated 32-byte client key on GameAuthRequest/GameResumeRequest.
	ClientPublicKey [32]byte
	// BuildID client build metadata on GameAuthRequest/GameResumeRequest.
	BuildID uint64
	// Capabilities client capability metadata on GameAuthRequest/GameResumeRequest; not authorization.
	Capabilities uint64
	// Encrypted resulting business mode on GameSecurityChanged; controls stay protected.
	Encrypted bool
	// SecurityEpoch GameSecurityChanged session-local key/mode generation, not a wall-clock time.
	SecurityEpoch uint64
	// SecurityOperation GameSecurityChanged RNET_SECURITY_OPERATION_* value.
	SecurityOperation SecurityOperation
	// QualityGrade GameQualityChanged grade: 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor.
	QualityGrade uint32
	// QualityBasis GameQualityChanged basis: 1 latency, 2 latency+UDP gaps, 3 latency+KCP retransmission.
	QualityBasis uint32
	// LastRTT GameQualityChanged latest authenticated heartbeat RTT as a time.Duration (native microsecond precision).
	LastRTT time.Duration
	// Jitter GameQualityChanged smoothed absolute RTT variation as a time.Duration (native microsecond precision).
	Jitter time.Duration
	// QualitySamples GameQualityChanged accepted heartbeat sample count.
	QualitySamples uint64
	// HasSequence whether GameMessage carries sender Sequence metadata.
	HasSequence bool
	// Sequence GameMessage sequence when HasSequence is true; not the internal UDP loss sequence.
	Sequence uint32
	// HasTick whether GameMessage carries a sender Tick.
	HasTick bool
	// Tick GameMessage simulation tick when HasTick is true; not trusted time.
	Tick uint32
	// CorrelationID GameMessage optional request/trace metadata; zero means absent.
	CorrelationID uint64
}

// GameQuality distinguishes unavailable TCP loss from a measured zero.
// UDP loss is a sequence-gap estimate; KCP reports retransmissions, not IP loss.
type GameQuality struct {
	// Available false before the first heartbeat sample; true permits interpreting measurements.
	Available bool
	// Grade 0 unknown, 1 excellent, 2 good, 3 fair, 4 poor; sample minima may keep grade unknown.
	Grade uint32
	// Basis 1 latency only, 2 latency+UDP gaps, 3 latency+KCP retransmissions.
	Basis uint32
	// HasUDPLoss whether UDP evidence is available; false does not mean measured zero loss.
	HasUDPLoss bool
	// HasKCPRetransmissions whether KCP retransmission evidence is available; TCP has neither transport signal.
	HasKCPRetransmissions bool
	// UDPRecentLossPerMille recent UDP sequence-gap estimate per 1,000 positions (10 means 1%); check availability.
	UDPRecentLossPerMille uint32
	// KCPRecentRetransmissionPerMille recent KCP retransmission ratio per 1,000 PUSH transmissions; not raw network loss.
	KCPRecentRetransmissionPerMille uint32
	// LastRTT latest authenticated heartbeat RTT as a time.Duration (native microsecond precision), including queueing effects.
	LastRTT time.Duration
	// SmoothedRTT smoothed heartbeat RTT as a time.Duration (native microsecond precision); not a percentile.
	SmoothedRTT time.Duration
	// Jitter smoothed absolute RTT variation as a time.Duration (native microsecond precision).
	Jitter time.Duration
	// Samples accepted heartbeat samples on this session.
	Samples uint64
	// UDPExpected cumulative expected UDP data positions, excluding implausible forward jumps.
	UDPExpected uint64
	// UDPMissing expected minus received UDP positions; late arrivals may reduce this value.
	UDPMissing uint64
	// KCPSegmentsSent cumulative KCP PUSH transmissions, including retransmissions, excluding ACKs.
	KCPSegmentsSent uint64
	// KCPRetransmitted cumulative repeated KCP PUSH transmissions.
	KCPRetransmitted uint64
}

// GameClockSync maps this client's runtime-local monotonic clock to the
// server's runtime-local monotonic clock. It is not UTC or a trusted time source.
type GameClockSync struct {
	// Available false on server sessions or before first client sample.
	Available bool
	// ServerMinusClient signed duration added to client monotonic clock to approximate the server runtime clock.
	ServerMinusClient time.Duration
	// RTT round-trip estimate after subtracting reported server turnaround; not trusted time.
	RTT time.Duration
	// Samples accepted clock samples, not heartbeat samples.
	Samples uint64
}

// String formats event routing metadata while redacting payloads and credentials.
func (event GameEvent) String() string {
	return fmt.Sprintf("GameEvent{Type:%d Endpoint:%d Session:%d RelatedSession:%d Status:%d DataLen:%d AuxDataLen:%d}",
		event.Type, event.Endpoint, event.Session, event.RelatedSession, event.Status,
		len(event.Data), len(event.AuxData))
}

// GoString uses the same redacted representation as String for Go-syntax formatting.
func (event GameEvent) GoString() string { return event.String() }
