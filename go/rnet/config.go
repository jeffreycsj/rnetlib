package rnet

import "time"

// Config controls bounded runtime queues and message sizes; initialize with DefaultConfig.
// It is copied at construction, not a live control object. A zero Config is invalid.
type Config struct {
	// WorkerThreads positive I/O worker count; initialize settings with DefaultConfig.
	WorkerThreads uint32
	// EventQueueCapacity positive transport event count limit; includes lifecycle events.
	EventQueueCapacity uint32
	// WriteQueueCapacity positive outbound command queue capacity in entries.
	WriteQueueCapacity uint32
	// MaxBodyLen positive logical frame body limit in bytes; game envelopes consume this budget too.
	MaxBodyLen uint32
	// MaxDatagramSize UDP/KCP datagram byte ceiling including protocol/security overhead; 20..65507, default 1200.
	MaxDatagramSize uint32
	// MaxEventBytes positive total queued event payload budget in bytes.
	MaxEventBytes uint64
	// MaxRuntimeQueuedBytes positive runtime-wide send byte budget including framing reservations.
	MaxRuntimeQueuedBytes uint64
	// MaxSessionQueuedBytes positive per-session send budget, at most MaxRuntimeQueuedBytes.
	MaxSessionQueuedBytes uint64
	// MaxEndpoints positive runtime endpoint cap, including listeners and outbound client endpoints.
	MaxEndpoints uint32
	// MaxPendingHandshakes positive runtime-wide pending-handshake admission cap.
	MaxPendingHandshakes uint32
	// MaxSessionsPerEndpoint positive active/pending session cap per listener/endpoint.
	MaxSessionsPerEndpoint uint32
	// MaxSessionsPerIP positive peer admission cap per IPv4 address or configured IPv6 prefix.
	MaxSessionsPerIP uint32
	// HandshakeRatePerIP positive handshake token refill rate per second and IP/prefix.
	HandshakeRatePerIP uint32
	// HandshakeBurstPerIP positive handshake token-bucket burst allowance per IP/prefix.
	HandshakeBurstPerIP uint32
	// IPv6AdmissionPrefixBits IPv6 admission prefix length, 0..128; zero groups all IPv6 peers, IPv4 uses full addresses.
	IPv6AdmissionPrefixBits uint32
	// HandshakeTimeout handshake/application authorization deadline; truncates to whole milliseconds and must remain positive.
	HandshakeTimeout time.Duration
	// ConnectTimeout TCP connection-attempt limit; truncates to whole milliseconds and must remain positive.
	ConnectTimeout time.Duration
	// DNSTimeout synchronous DNS wait limit; truncates to whole milliseconds and must remain positive.
	DNSTimeout time.Duration
	// TCPNoDelay enables TCP_NODELAY; true in DefaultConfig.
	TCPNoDelay bool
	// TCPSendBufferBytes requested TCP kernel send buffer bytes; zero keeps OS defaults.
	TCPSendBufferBytes uint64
	// TCPRecvBufferBytes requested TCP kernel receive buffer bytes; zero keeps OS defaults.
	TCPRecvBufferBytes uint64
	// DatagramIdleTimeout established UDP/KCP idle timeout; truncates to whole milliseconds and must remain positive.
	DatagramIdleTimeout time.Duration
	// AllowPlaintextBusinessData permits local server plaintext business mode; clients follow server policy. Overridden by GameConfig when nested there.
	AllowPlaintextBusinessData bool
	// RekeyAfter server automatic key-rotation age; truncated to milliseconds. Nonpositive/sub-millisecond values disable this trigger.
	RekeyAfter time.Duration
	// RekeyAfterBytes server automatic protected-byte rotation threshold; zero disables this trigger.
	RekeyAfterBytes uint64
	// Logger optional asynchronous sink; serial on one runtime, potentially concurrent across runtimes. Return quickly; never Stop/Close from it. Panics are recovered.
	Logger func(LogRecord)
	// MinLogLevel minimum inclusive severity; DefaultConfig uses LogInfo. Values above LogError clamp to LogError.
	MinLogLevel LogLevel
	// ClientSecurity optional copied client identity/server pin; nil gives server-only usage. Must be nil inside GameConfig.Network.
	ClientSecurity *ClientSecurity
}

// ClientSecurity supplies runtime-wide client identity and an exact server key pin.
type ClientSecurity struct {
	// LocalKey client identity copied at runtime creation; caller owns erasure of this and any other private-key copies.
	LocalKey Keypair
	// ExpectedServerPublicKey exactly 32 pinned server public-key bytes, copied at runtime creation.
	ExpectedServerPublicKey []byte
}

// DefaultConfig returns production-oriented bounded defaults.
func DefaultConfig() Config {
	return Config{
		WorkerThreads:           2,
		EventQueueCapacity:      4096,
		WriteQueueCapacity:      256,
		MaxBodyLen:              1024 * 1024,
		MaxDatagramSize:         1200,
		MaxEventBytes:           64 * 1024 * 1024,
		MaxRuntimeQueuedBytes:   256 * 1024 * 1024,
		MaxSessionQueuedBytes:   4 * 1024 * 1024,
		MaxEndpoints:            1024,
		MaxPendingHandshakes:    16_384,
		MaxSessionsPerEndpoint:  16_384,
		MaxSessionsPerIP:        256,
		HandshakeRatePerIP:      100,
		HandshakeBurstPerIP:     200,
		IPv6AdmissionPrefixBits: 64,
		HandshakeTimeout:        5 * time.Second,
		ConnectTimeout:          5 * time.Second,
		DNSTimeout:              5 * time.Second,
		TCPNoDelay:              true,
		DatagramIdleTimeout:     120 * time.Second,
		RekeyAfter:              time.Hour,
		RekeyAfterBytes:         1024 * 1024 * 1024,
		MinLogLevel:             LogInfo,
	}
}
