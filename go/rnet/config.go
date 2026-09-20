package rnet

import "time"

// Config controls bounded runtime queues and message sizes.
type Config struct {
	WorkerThreads              uint32
	EventQueueCapacity         uint32
	WriteQueueCapacity         uint32
	MaxBodyLen                 uint32
	MaxDatagramSize            uint32
	MaxEventBytes              uint64
	MaxRuntimeQueuedBytes      uint64
	MaxSessionQueuedBytes      uint64
	MaxEndpoints               uint32
	MaxPendingHandshakes       uint32
	MaxSessionsPerEndpoint     uint32
	MaxSessionsPerIP           uint32
	HandshakeRatePerIP         uint32
	HandshakeBurstPerIP        uint32
	IPv6AdmissionPrefixBits    uint32
	HandshakeTimeout           time.Duration
	ConnectTimeout             time.Duration
	DNSTimeout                 time.Duration
	TCPNoDelay                 bool
	TCPSendBufferBytes         uint64
	TCPRecvBufferBytes         uint64
	DatagramIdleTimeout        time.Duration
	AllowPlaintextBusinessData bool
	AllowLegacyEndpoints       bool
	RekeyAfter                 time.Duration
	RekeyAfterBytes            uint64
	// Logger may be called concurrently. It must return quickly; RNet drops logs under pressure.
	Logger      func(LogRecord)
	MinLogLevel LogLevel
	// ClientSecurity is runtime-scoped. Connect never chooses an encryption mode.
	ClientSecurity *ClientSecurity
}

// ClientSecurity supplies runtime-wide client identity and an exact server key pin.
type ClientSecurity struct {
	LocalKey                Keypair
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
