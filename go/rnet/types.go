package rnet

/*
#include "native.h"
*/
import "C"

// Endpoint is a runtime-local listener or outbound-connection handle, not a send destination.
// Zero is invalid. Never use a handle with another runtime or after its endpoint closes.
type Endpoint uint64

// Session is a runtime-local peer handle used for authorization, sends and security changes.
// Zero is invalid. A successful resume replaces the old handle; do not reuse closed handles.
type Session uint64

// EventType identifies a transport event and determines which Event fields are meaningful.
type EventType uint32

// LatencyKind selects the measured operation; values from different kinds are not interchangeable.
type LatencyKind uint32

// LogLevel orders severity from LogTrace through LogError; higher values are less verbose.
type LogLevel uint32

// Transport selects the immutable endpoint transport.
type Transport uint32

// SecurityMode describes business-record protection; control traffic remains authenticated.
type SecurityMode uint32

// SecurityOperation identifies which authenticated barrier completed in a security-change event.
type SecurityOperation uint8

const (
	// LogTrace — Most verbose diagnostics.
	LogTrace LogLevel = C.RNET_LOG_TRACE
	// LogDebug — Development/debug diagnostics.
	LogDebug LogLevel = C.RNET_LOG_DEBUG
	// LogInfo — Normal lifecycle/summary information.
	LogInfo LogLevel = C.RNET_LOG_INFO
	// LogWarn — Recoverable issue or anomalous peer behavior.
	LogWarn LogLevel = C.RNET_LOG_WARN
	// LogError — Operation/endpoint/runtime failure.
	LogError LogLevel = C.RNET_LOG_ERROR
)

// LogRecord is emitted by the bounded structured logger without payload or key material.
type LogRecord struct {
	// TimestampUnixMillis UTC timestamp in milliseconds.
	TimestampUnixMillis uint64
	// Level record severity.
	Level LogLevel
	// EventName owned structured event name.
	EventName string
	// Runtime diagnostic native runtime handle; not a Go object pointer.
	Runtime uint64
	// Endpoint local source endpoint; zero unspecified.
	Endpoint Endpoint
	// Session local source session; zero unspecified.
	Session Session
	// Transport source transport; zero unspecified.
	Transport Transport
	// ErrorCode local native error category; zero means no error.
	ErrorCode int32
	// CorrelationID optional request/trace ID; zero absent.
	CorrelationID uint64
	// Message owned diagnostic text; not a business payload or credential.
	Message string
}

const (
	// SecurityOperationModeSwitch — Business encryption policy barrier completed locally.
	SecurityOperationModeSwitch SecurityOperation = C.RNET_SECURITY_OPERATION_MODE_SWITCH
	// SecurityOperationRekey — Key-generation barrier completed locally.
	SecurityOperationRekey SecurityOperation = C.RNET_SECURITY_OPERATION_REKEY
)

const (
	// TransportTCP — Reliable ordered stream transport with library message framing.
	TransportTCP Transport = C.RNET_TRANSPORT_TCP
	// TransportUDP — Unreliable message transport; no retransmission or delivery guarantee.
	TransportUDP Transport = C.RNET_TRANSPORT_UDP
	// TransportKCP — Reliable ordered KCP delivery over UDP.
	TransportKCP Transport = C.RNET_TRANSPORT_KCP
)

const (
	// SecurityPlaintext — Unprotected business bytes; protected controls and peer handshake remain enabled.
	SecurityPlaintext SecurityMode = C.RNET_SECURITY_PLAINTEXT
	// SecurityEncrypted — Authenticated encrypted business bytes and protected controls.
	SecurityEncrypted SecurityMode = C.RNET_SECURITY_ENCRYPTED
)

// ServerConfig contains the transport and initial server-controlled security policy.
type ServerConfig struct {
	// Transport fixed listener transport; choose TransportTCP/UDP/KCP, zero is invalid.
	Transport Transport
	// Host numeric bind IP, without brackets, scheme or port; DNS is not accepted.
	Host string
	// Port local bind port; zero requests an OS-assigned port.
	Port uint16
	// Keypair server identity copied by Listen; use a stable key when clients pin it. Caller erases copies.
	Keypair Keypair
	// InitialSecurity required SecurityEncrypted/Plaintext; zero is invalid for the transport API. Plaintext requires runtime opt-in.
	InitialSecurity SecurityMode
}

// ClientConfig contains connection routing and application join data, but no encryption choice.
type ClientConfig struct {
	// Transport fixed transport matching the listener; zero invalid.
	Transport Transport
	// Host DNS name or numeric IP, without scheme/path or embedded port.
	Host string
	// Port remote service port; must be nonzero.
	Port uint16
	// JoinPayload opaque protected join bytes; copied by Connect, empty permitted if authorization allows.
	JoinPayload []byte
}

const (
	// EventRuntimeStarted — Runtime started; endpoint/session zero.
	EventRuntimeStarted EventType = C.RNET_EVENT_RUNTIME_STARTED
	// EventEndpointOpened — Endpoint socket became available; not a ready session.
	EventEndpointOpened EventType = C.RNET_EVENT_ENDPOINT_OPENED
	// EventEndpointError — Endpoint-related failure; may be recoverable, query endpoint before assuming closure.
	EventEndpointError EventType = C.RNET_EVENT_ENDPOINT_ERROR
	// EventSessionOpened — Transport handshake and authorization completed; not game readiness.
	EventSessionOpened EventType = C.RNET_EVENT_SESSION_OPENED
	// EventSessionClosed — Peer closed; status is local reason, data may contain a diagnostic.
	EventSessionClosed EventType = C.RNET_EVENT_SESSION_CLOSED
	// EventMessage — Opaque transport payload with optional correlation metadata.
	EventMessage EventType = C.RNET_EVENT_MESSAGE
	// EventWritable — Advisory send-capacity recovery; another send can still encounter backpressure.
	EventWritable EventType = C.RNET_EVENT_WRITABLE
	// EventRuntimeStopped — Runtime stop completed; release remaining buffers before destroy.
	EventRuntimeStopped EventType = C.RNET_EVENT_RUNTIME_STOPPED
	// EventAuthRequest — Server must decide; data contains key[32] followed by opaque join payload.
	EventAuthRequest EventType = C.RNET_EVENT_AUTH_REQUEST
	// EventJoinFailed — Outbound connection/handshake failed; status/detail explain the failure.
	EventJoinFailed EventType = C.RNET_EVENT_JOIN_FAILED
	// EventSecurityChanged — Local barrier committed; data has mode, big-endian epoch, operation (10 bytes).
	EventSecurityChanged EventType = C.RNET_EVENT_SECURITY_CHANGED
)

const (
	// LatencyConnect — TCP connect operation duration.
	LatencyConnect LatencyKind = C.RNET_LATENCY_CONNECT
	// LatencyCryptoHandshake — Cryptographic handshake duration.
	LatencyCryptoHandshake LatencyKind = C.RNET_LATENCY_CRYPTO_HANDSHAKE
	// LatencyAuthWait — Server application authorization wait duration.
	LatencyAuthWait LatencyKind = C.RNET_LATENCY_AUTH_WAIT
	// LatencySendQueue — Time waiting for transport send processing.
	LatencySendQueue LatencyKind = C.RNET_LATENCY_SEND_QUEUE
	// LatencyEventQueue — Time waiting for application event polling.
	LatencyEventQueue LatencyKind = C.RNET_LATENCY_EVENT_QUEUE
	// LatencyKCPRTT — KCP acknowledgement-based RTT samples, not game heartbeat RTT.
	LatencyKCPRTT LatencyKind = C.RNET_LATENCY_KCP_RTT
	// LatencyKCPUpdateDelay — Observed KCP update scheduling delay.
	LatencyKCPUpdateDelay LatencyKind = C.RNET_LATENCY_KCP_UPDATE_DELAY
	// LatencyLoggerCallback — Time spent executing the installed log callback.
	LatencyLoggerCallback LatencyKind = C.RNET_LATENCY_LOGGER_CALLBACK
)
