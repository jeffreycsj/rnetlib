package rnet

/*
#include "native.h"
*/
import "C"

type Endpoint uint64
type Session uint64
type EventType uint32
type LatencyKind uint32
type LogLevel uint32

// Transport selects the immutable endpoint transport.
type Transport uint32

// SecurityMode describes business-record protection; control traffic remains authenticated.
type SecurityMode uint32
type SecurityOperation uint8

const (
	LogTrace LogLevel = C.RNET_LOG_TRACE
	LogDebug LogLevel = C.RNET_LOG_DEBUG
	LogInfo  LogLevel = C.RNET_LOG_INFO
	LogWarn  LogLevel = C.RNET_LOG_WARN
	LogError LogLevel = C.RNET_LOG_ERROR
)

// LogRecord is emitted by the bounded structured logger without payload or key material.
type LogRecord struct {
	TimestampUnixMillis uint64
	Level               LogLevel
	EventName           string
	Runtime             uint64
	Endpoint            Endpoint
	Session             Session
	Transport           Transport
	ErrorCode           int32
	CorrelationID       uint64
	Message             string
}

const (
	SecurityOperationModeSwitch SecurityOperation = C.RNET_SECURITY_OPERATION_MODE_SWITCH
	SecurityOperationRekey      SecurityOperation = C.RNET_SECURITY_OPERATION_REKEY
)

const (
	TransportTCP Transport = C.RNET_TRANSPORT_TCP
	TransportUDP Transport = C.RNET_TRANSPORT_UDP
	TransportKCP Transport = C.RNET_TRANSPORT_KCP
)

const (
	SecurityPlaintext SecurityMode = C.RNET_SECURITY_PLAINTEXT
	SecurityEncrypted SecurityMode = C.RNET_SECURITY_ENCRYPTED
)

// ServerConfig contains the transport and initial server-controlled security policy.
type ServerConfig struct {
	Transport       Transport
	Host            string
	Port            uint16
	Keypair         Keypair
	InitialSecurity SecurityMode
}

// ClientConfig contains connection routing and application join data, but no encryption choice.
type ClientConfig struct {
	Transport   Transport
	Host        string
	Port        uint16
	JoinPayload []byte
}

const (
	EventRuntimeStarted  EventType = C.RNET_EVENT_RUNTIME_STARTED
	EventEndpointOpened  EventType = C.RNET_EVENT_ENDPOINT_OPENED
	EventEndpointError   EventType = C.RNET_EVENT_ENDPOINT_ERROR
	EventSessionOpened   EventType = C.RNET_EVENT_SESSION_OPENED
	EventSessionClosed   EventType = C.RNET_EVENT_SESSION_CLOSED
	EventMessage         EventType = C.RNET_EVENT_MESSAGE
	EventWritable        EventType = C.RNET_EVENT_WRITABLE
	EventRuntimeStopped  EventType = C.RNET_EVENT_RUNTIME_STOPPED
	EventAuthRequest     EventType = C.RNET_EVENT_AUTH_REQUEST
	EventJoinFailed      EventType = C.RNET_EVENT_JOIN_FAILED
	EventSecurityChanged EventType = C.RNET_EVENT_SECURITY_CHANGED
)

const (
	LatencyConnect         LatencyKind = C.RNET_LATENCY_CONNECT
	LatencyCryptoHandshake LatencyKind = C.RNET_LATENCY_CRYPTO_HANDSHAKE
	LatencyAuthWait        LatencyKind = C.RNET_LATENCY_AUTH_WAIT
	LatencySendQueue       LatencyKind = C.RNET_LATENCY_SEND_QUEUE
	LatencyEventQueue      LatencyKind = C.RNET_LATENCY_EVENT_QUEUE
	LatencyKCPRTT          LatencyKind = C.RNET_LATENCY_KCP_RTT
	LatencyKCPUpdateDelay  LatencyKind = C.RNET_LATENCY_KCP_UPDATE_DELAY
	LatencyLoggerCallback  LatencyKind = C.RNET_LATENCY_LOGGER_CALLBACK
)
