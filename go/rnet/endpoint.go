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
	status := C.rnet_go_server_listen(
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
	status := C.rnet_go_client_connect(
		handle, C.uint32_t(config.Transport), bytePointer(host), C.size_t(len(host)),
		C.uint16_t(config.Port), bytePointer(config.JoinPayload),
		C.size_t(len(config.JoinPayload)), &endpoint,
	)
	runtime.KeepAlive(config)
	return Endpoint(endpoint), statusError(status)
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
