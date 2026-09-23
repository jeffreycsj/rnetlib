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

// GameProtocolRange opts into authenticated wire-v4 version negotiation.
// Exact-version GameProtocol connections retain wire v3.
type GameProtocolRange struct {
	ID           uint64
	MinVersion   uint32
	MaxVersion   uint32
	BuildID      uint64
	Capabilities uint64
}

type GameRangeServerConfig struct {
	Transport       Transport
	Host            string
	Port            uint16
	Keypair         Keypair
	InitialSecurity SecurityMode
	Protocol        GameProtocolRange
}

func (config GameRangeServerConfig) String() string {
	return fmt.Sprintf("GameRangeServerConfig{Transport:%d Host:%q Port:%d InitialSecurity:%d ProtocolID:%d Range:[%d,%d] Keypair:<redacted>}",
		config.Transport, config.Host, config.Port, config.InitialSecurity, config.Protocol.ID, config.Protocol.MinVersion, config.Protocol.MaxVersion)
}

func (config GameRangeServerConfig) GoString() string { return config.String() }

type GameRangeClientConfig struct {
	Transport  Transport
	Host       string
	Port       uint16
	JoinTicket []byte
	Protocol   GameProtocolRange
}

// GameTransportLatest is cumulative and runtime-wide. WorkerPickups marks the
// point after which a TCP/KCP snapshot can no longer be replaced.
type GameTransportLatest struct {
	PendingReplaced            uint64
	WorkerPickups              uint64
	AdmissionWouldBlock        uint64
	AdmissionInvalidHandle     uint64
	AdmissionInvalidState      uint64
	AdmissionHandshakeRequired uint64
	AdmissionNotSupported      uint64
	AdmissionMessageTooLarge   uint64
	AdmissionOtherFailures     uint64
}

func (config GameRangeClientConfig) String() string {
	return fmt.Sprintf("GameRangeClientConfig{Transport:%d Host:%q Port:%d ProtocolID:%d Range:[%d,%d] JoinTicketLen:%d}",
		config.Transport, config.Host, config.Port, config.Protocol.ID, config.Protocol.MinVersion, config.Protocol.MaxVersion, len(config.JoinTicket))
}

func (config GameRangeClientConfig) GoString() string { return config.String() }

func (r *GameRuntime) ListenRange(config GameRangeServerConfig) (Endpoint, error) {
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
	status := C.rnet_go_game_server_listen_range(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), (*C.uint8_t)(unsafe.Pointer(&config.Keypair.Private[0])),
		encrypted, C.uint64_t(config.Protocol.ID), C.uint32_t(config.Protocol.MinVersion),
		C.uint32_t(config.Protocol.MaxVersion), &endpoint,
	)
	runtime.KeepAlive(config)
	runtime.KeepAlive(host)
	return Endpoint(endpoint), statusError(status)
}

func (r *GameRuntime) ConnectRange(config GameRangeClientConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_client_connect_range(
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

func (r *GameRuntime) ConnectRangeResume(config GameRangeClientConfig, oldSession Session, ticket []byte) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_game_client_resume_connect_range(
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
