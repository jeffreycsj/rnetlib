package rnet

/*
#include "native.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

// Listen opens a server endpoint. The transport is selected by config, not by the method name.
func (r *Runtime) Listen(config ServerConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_server_open_v2(
		handle, C.uint32_t(config.Transport), C.uint32_t(config.InitialSecurity),
		bytePointer(host), C.size_t(len(host)), C.uint16_t(config.Port),
		(*C.uint8_t)(unsafe.Pointer(&config.Keypair.Private[0])), &endpoint,
	)
	runtime.KeepAlive(config)
	return Endpoint(endpoint), statusError(status)
}

// Connect starts a client connection; encryption is negotiated and handled internally.
func (r *Runtime) Connect(config ClientConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_client_connect_v2(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinPayload),
		C.size_t(len(config.JoinPayload)), &endpoint,
	)
	runtime.KeepAlive(config)
	return Endpoint(endpoint), statusError(status)
}

func (r *Runtime) OpenTCPListener(host string, port uint16) (Endpoint, error) {
	return r.openEndpoint(C.RNET_TRANSPORT_TCP, C.RNET_ENDPOINT_LISTENER, host, port, "", 0)
}

func (r *Runtime) OpenTCPClient(host string, port uint16) (Endpoint, error) {
	return r.openEndpoint(C.RNET_TRANSPORT_TCP, C.RNET_ENDPOINT_CLIENT, "", 0, host, port)
}

func (r *Runtime) OpenUDP(bindHost string, bindPort uint16, remoteHost string, remotePort uint16) (Endpoint, error) {
	return r.openEndpoint(C.RNET_TRANSPORT_UDP, C.RNET_ENDPOINT_DATAGRAM, bindHost, bindPort, remoteHost, remotePort)
}

func (r *Runtime) OpenSecureTCPListener(host string, port uint16, keypair Keypair) (Endpoint, error) {
	return r.openSecureListener(C.RNET_TRANSPORT_TCP, host, port, keypair)
}

func (r *Runtime) OpenSecureUDPListener(host string, port uint16, keypair Keypair) (Endpoint, error) {
	return r.openSecureListener(C.RNET_TRANSPORT_UDP, host, port, keypair)
}

func (r *Runtime) OpenSecureKCPListener(host string, port uint16, keypair Keypair) (Endpoint, error) {
	return r.openSecureListener(C.RNET_TRANSPORT_KCP, host, port, keypair)
}

func (r *Runtime) openSecureListener(transport C.uint32_t, host string, port uint16, keypair Keypair) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	hostBytes := []byte(host)
	var endpoint C.rnet_endpoint_t
	callErr := statusError(C.rnet_go_listener_open(
		handle, transport, bytePointer(hostBytes), C.size_t(len(hostBytes)),
		C.uint16_t(port), (*C.uint8_t)(unsafe.Pointer(&keypair.Private[0])), &endpoint,
	))
	runtime.KeepAlive(hostBytes)
	runtime.KeepAlive(keypair)
	return Endpoint(endpoint), callErr
}

func (r *Runtime) JoinSecureTCP(host string, port uint16, localKey Keypair, serverPublicKey, payload []byte) (Endpoint, error) {
	return r.joinSecure(C.RNET_TRANSPORT_TCP, host, port, localKey, serverPublicKey, payload)
}

func (r *Runtime) JoinSecureUDP(host string, port uint16, localKey Keypair, serverPublicKey, payload []byte) (Endpoint, error) {
	return r.joinSecure(C.RNET_TRANSPORT_UDP, host, port, localKey, serverPublicKey, payload)
}

func (r *Runtime) JoinSecureKCP(host string, port uint16, localKey Keypair, serverPublicKey, payload []byte) (Endpoint, error) {
	return r.joinSecure(C.RNET_TRANSPORT_KCP, host, port, localKey, serverPublicKey, payload)
}

func (r *Runtime) joinSecure(transport C.uint32_t, host string, port uint16, localKey Keypair, serverPublicKey, payload []byte) (Endpoint, error) {
	defer lockNativeThread()()
	if len(serverPublicKey) != 32 {
		return 0, StatusError{Code: int32(C.RNET_E_INVALID_ARGUMENT), Message: "invalid endpoint configuration"}
	}
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	hostBytes := []byte(host)
	var endpoint C.rnet_endpoint_t
	callErr := statusError(C.rnet_go_client_join(
		handle, transport, bytePointer(hostBytes), C.size_t(len(hostBytes)),
		C.uint16_t(port), (*C.uint8_t)(unsafe.Pointer(&localKey.Private[0])),
		bytePointer(serverPublicKey), bytePointer(payload), C.size_t(len(payload)), &endpoint,
	))
	runtime.KeepAlive(hostBytes)
	runtime.KeepAlive(localKey)
	runtime.KeepAlive(serverPublicKey)
	runtime.KeepAlive(payload)
	return Endpoint(endpoint), callErr
}

func (r *Runtime) openEndpoint(transport, mode C.uint32_t, bindHost string, bindPort uint16, remoteHost string, remotePort uint16) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	bindBytes := []byte(bindHost)
	remoteBytes := []byte(remoteHost)
	var endpoint C.rnet_endpoint_t
	callErr := statusError(C.rnet_go_endpoint_open(
		handle,
		transport,
		mode,
		bytePointer(bindBytes),
		C.size_t(len(bindBytes)),
		C.uint16_t(bindPort),
		bytePointer(remoteBytes),
		C.size_t(len(remoteBytes)),
		C.uint16_t(remotePort),
		&endpoint,
	))
	runtime.KeepAlive(bindBytes)
	runtime.KeepAlive(remoteBytes)
	return Endpoint(endpoint), callErr
}

func (r *Runtime) LocalPort(endpoint Endpoint) (uint16, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	var port C.uint16_t
	err = statusError(C.rnet_endpoint_local_port(handle, C.rnet_endpoint_t(endpoint), &port))
	return uint16(port), err
}

func (r *Runtime) CloseEndpoint(endpoint Endpoint) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_endpoint_close(handle, C.rnet_endpoint_t(endpoint)))
}
