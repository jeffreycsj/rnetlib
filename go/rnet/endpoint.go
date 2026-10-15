package rnet

/*
#include "native.h"
*/
import "C"

import (
	"runtime"
	"unsafe"
)

// Listen binds a numeric IP using config's fixed transport and copies its server key.
// Port zero requests an assigned port, readable with LocalPort. A returned endpoint
// is not a session: Poll, authorize EventAuthRequest, then await EventSessionOpened.
func (r *Runtime) Listen(config ServerConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_server_listen(
		handle, C.uint32_t(config.Transport), C.uint32_t(config.InitialSecurity),
		bytePointer(host), C.size_t(len(host)), C.uint16_t(config.Port),
		(*C.uint8_t)(unsafe.Pointer(&config.Keypair.Private[0])), &endpoint,
	)
	runtime.KeepAlive(config)
	return Endpoint(endpoint), statusError(status)
}

// Connect starts an authenticated connection using the constructor's ClientSecurity.
// Host accepts a DNS name or numeric IP, not a URL; DNS resolution can block up to
// the configured timeout. The returned endpoint is not ready: await EventSessionOpened.
// JoinPayload is copied; failures may also arrive asynchronously as EventJoinFailed.
func (r *Runtime) Connect(config ClientConfig) (Endpoint, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, err
	}
	host := []byte(config.Host)
	var endpoint C.rnet_endpoint_t
	status := C.rnet_go_client_connect(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinPayload),
		C.size_t(len(config.JoinPayload)), &endpoint,
	)
	runtime.KeepAlive(config)
	return Endpoint(endpoint), statusError(status)
}

// LocalPort returns the bound port for a live endpoint owned by this runtime.
// It is useful after Listen with port zero; stale or foreign handles are invalid.
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

// CloseEndpoint closes the given local endpoint and its sessions, including a listener.
// Previously returned session handles must no longer be used for sends.
func (r *Runtime) CloseEndpoint(endpoint Endpoint) error {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	return statusError(C.rnet_endpoint_close(handle, C.rnet_endpoint_t(endpoint)))
}
