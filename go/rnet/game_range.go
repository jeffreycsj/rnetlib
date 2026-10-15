package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"runtime"
	"unsafe"
)

// GameProtocol describes authenticated application version negotiation.
// Bounds are inclusive and nonzero; equal bounds require that exact version.
type GameProtocol struct {
	// ID nonzero business protocol ID matching the server, independent of SDK/wire version.
	ID uint64
	// MinVersion positive inclusive minimum business version.
	MinVersion uint32
	// MaxVersion inclusive maximum version, at least MinVersion; equal bounds require exact match, highest overlap selected.
	MaxVersion uint32
	// BuildID optional authenticated client build metadata; zero unspecified, ignored by Listen.
	BuildID uint64
	// Capabilities application-defined authenticated client bits; zero none, ignored by Listen, not interpreted as library grants.
	Capabilities uint64
}

// GameServerConfig selects a fixed transport and authenticated application version range.
// Listen copies it. Zero InitialSecurity means encrypted; other required fields need explicit values.
type GameServerConfig struct {
	// Transport fixed listener transport; zero invalid.
	Transport Transport
	// Host numeric bind IP without brackets, URL scheme or embedded port.
	Host string
	// Port local port; zero requests an OS-assigned port.
	Port uint16
	// Keypair server identity copied by Listen; caller owns erasure of secret copies.
	Keypair Keypair
	// InitialSecurity zero selects encrypted business data; SecurityPlaintext requires GameConfig server opt-in.
	InitialSecurity SecurityMode
	// Protocol required nonzero ID/version bounds; default zero fields are invalid.
	Protocol GameProtocol
}

// String formats routing metadata without exposing identity keys or join credentials.
func (config GameServerConfig) String() string {
	return fmt.Sprintf("GameServerConfig{Transport:%d Host:%q Port:%d InitialSecurity:%d ProtocolID:%d Range:[%d,%d] Keypair:<redacted>}",
		config.Transport, config.Host, config.Port, config.InitialSecurity, config.Protocol.ID, config.Protocol.MinVersion, config.Protocol.MaxVersion)
}

// GoString uses String's redacted format for Go-syntax formatting.
func (config GameServerConfig) GoString() string { return config.String() }

// GameClientConfig selects a remote endpoint and protected join metadata.
// Connect copies it; encryption follows the server, not a client preference.
type GameClientConfig struct {
	// Transport fixed transport matching listener; zero invalid.
	Transport Transport
	// Host DNS name or numeric IP without scheme/path or embedded port.
	Host string
	// Port remote service port; must be nonzero.
	Port uint16
	// JoinTicket opaque protected login bytes copied by Connect; empty allowed if application authorization permits.
	JoinTicket []byte
	// Protocol required business ID/version bounds and optional client build/capability metadata.
	Protocol GameProtocol
}

// GameTransportLatest is cumulative and runtime-wide. WorkerPickups marks the
// point after which a TCP/KCP snapshot can no longer be replaced.
type GameTransportLatest struct {
	// PendingReplaced cumulative TCP/KCP pending values replaced before I/O worker pickup.
	PendingReplaced uint64
	// WorkerPickups cumulative slots consumed by I/O workers; no recall is possible after pickup.
	WorkerPickups uint64
	// AdmissionWouldBlock cumulative bounded transport-capacity rejections, including retries.
	AdmissionWouldBlock uint64
	// AdmissionInvalidHandle cumulative admissions referencing retired/unknown sessions.
	AdmissionInvalidHandle uint64
	// AdmissionInvalidState cumulative admissions when runtime/session was not accepting work.
	AdmissionInvalidState uint64
	// AdmissionHandshakeRequired cumulative admissions before transport readiness.
	AdmissionHandshakeRequired uint64
	// AdmissionNotSupported cumulative unsupported-path rejections; UDP uses ordinary forwarding.
	AdmissionNotSupported uint64
	// AdmissionMessageTooLarge cumulative size-limit rejections.
	AdmissionMessageTooLarge uint64
	// AdmissionOtherFailures cumulative admission failures outside the explicit categories.
	AdmissionOtherFailures uint64
}

// String formats routing metadata without exposing identity keys or join credentials.
func (config GameClientConfig) String() string {
	return fmt.Sprintf("GameClientConfig{Transport:%d Host:%q Port:%d ProtocolID:%d Range:[%d,%d] JoinTicketLen:%d}",
		config.Transport, config.Host, config.Port, config.Protocol.ID, config.Protocol.MinVersion, config.Protocol.MaxVersion, len(config.JoinTicket))
}

// GoString uses String's redacted format for Go-syntax formatting.
func (config GameClientConfig) GoString() string { return config.String() }

// Listen binds config.Host (numeric IP only) using the fixed transport and copied server key.
// Port zero requests an assigned port. Poll and AuthDecide pending joins, then wait for
// GameSessionReady; listener creation does not make any session ready.
func (r *GameRuntime) Listen(config GameServerConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var encrypted C.uint32_t = 1
	if config.InitialSecurity == SecurityPlaintext {
		encrypted = 0
	} else if config.InitialSecurity != 0 && config.InitialSecurity != SecurityEncrypted {
		return 0, fmt.Errorf("rnet: invalid game initial security mode")
	}
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_server_listen(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), (*C.uint8_t)(unsafe.Pointer(&config.Keypair.Private[0])),
		encrypted, C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.MinVersion),
		C.uint32_t(config.Protocol.MaxVersion), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	return Endpoint(endpoint), statusError(status)
}

// Connect starts a fresh authenticated game join using constructor-supplied client security.
// Host accepts a DNS name or IP, not a URL; resolution can block up to the DNS timeout.
// The returned endpoint is not a ready session. Poll until GameSessionReady or a failure.
func (r *GameRuntime) Connect(config GameClientConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_client_connect(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinTicket), C.size_t(len(config.JoinTicket)),
		C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.MinVersion),
		C.uint32_t(config.Protocol.MaxVersion), C.uint64_t(config.Protocol.BuildID),
		C.uint64_t(config.Protocol.Capabilities), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	return Endpoint(endpoint), statusError(status)
}

// ConnectResume starts a new connection using a copied one-use ticket and fresh join metadata.
// oldSession identifies the old local handle; it must be nonzero. Reuse the original
// client identity and a version range containing the ticket's selected version.
// The same server runtime must reauthorize. Only GameSessionResumed confirms takeover
// and maps old to new; failed negotiation does not retire a still-live old session.
func (r *GameRuntime) ConnectResume(config GameClientConfig, oldSession Session, ticket []byte) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_client_resume_connect(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinTicket), C.size_t(len(config.JoinTicket)),
		C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.MinVersion),
		C.uint32_t(config.Protocol.MaxVersion), C.uint64_t(config.Protocol.BuildID),
		C.uint64_t(config.Protocol.Capabilities), C.rnet_session_t(oldSession),
		bytePointer(ticket), C.size_t(len(ticket)), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	runtime.KeepAlive(ticket)
	return Endpoint(endpoint), statusError(status)
}

// SelectedProtocolVersion returns the negotiated business version, including pending server auth.
// An unknown session or absent selection fails. Selection is not proof of authorization
// or game readiness, and is independent of the SDK/wire version.
func (r *GameRuntime) SelectedProtocolVersion(session Session) (uint32, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	var version C.uint32_t
	status := C.rnet_game_selected_protocol_version(handle, C.rnet_session_t(session), &version)
	return uint32(version), statusError(status)
}

// TransportLatestReplacements returns the runtime's cumulative TCP/KCP pending-slot replacement count.
// It excludes game-staging replacements and is not a count of delivered snapshots.
func (r *GameRuntime) TransportLatestReplacements() (uint64, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	var replaced C.uint64_t
	status := C.rnet_game_transport_latest_replacements(handle, &replaced)
	return uint64(replaced), statusError(status)
}

// TransportLatestSnapshot copies cumulative TCP/KCP pending-slot counters without resetting them.
// WorkerPickups marks the point beyond which a snapshot cannot be replaced.
func (r *GameRuntime) TransportLatestSnapshot() (GameTransportLatest, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return GameTransportLatest{}, err
	}
	var raw C.rnet_game_transport_latest_t
	status := C.rnet_game_transport_latest_snapshot(handle, &raw)
	if err := statusError(status); err != nil {
		return GameTransportLatest{}, err
	}
	return GameTransportLatest{
		PendingReplaced: uint64(raw.pending_replaced), WorkerPickups: uint64(raw.worker_pickups),
		AdmissionWouldBlock:        uint64(raw.admission_would_block),
		AdmissionInvalidHandle:     uint64(raw.admission_invalid_handle),
		AdmissionInvalidState:      uint64(raw.admission_invalid_state),
		AdmissionHandshakeRequired: uint64(raw.admission_handshake_required),
		AdmissionNotSupported:      uint64(raw.admission_not_supported),
		AdmissionMessageTooLarge:   uint64(raw.admission_message_too_large),
		AdmissionOtherFailures:     uint64(raw.admission_other_failures),
	}, nil
}
