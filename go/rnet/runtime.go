package rnet

/*
#include "native.h"
*/
import "C"

import (
	"fmt"
	"runtime"
	"runtime/cgo"
	"sync"
	"time"
	"unsafe"
)

type Runtime struct {
	mu      sync.RWMutex
	handle  C.rnet_runtime_t
	stopped bool
	logger  cgo.Handle
}

func NewRuntime(configs ...Config) (*Runtime, error) {
	if len(configs) > 1 {
		return nil, fmt.Errorf("rnet: NewRuntime accepts at most one config")
	}
	config := DefaultConfig()
	if len(configs) == 1 {
		config = configs[0]
	}
	return NewRuntimeWithConfig(config)
}

func NewRuntimeWithConfig(config Config) (*Runtime, error) {
	defer lockNativeThread()()
	var native C.rnet_config_v5_t
	if err := statusError(C.rnet_config_v5_init(&native)); err != nil {
		return nil, err
	}
	native.worker_threads = C.uint32_t(config.WorkerThreads)
	native.event_queue_capacity = C.uint32_t(config.EventQueueCapacity)
	native.write_queue_capacity = C.uint32_t(config.WriteQueueCapacity)
	native.max_body_len = C.uint32_t(config.MaxBodyLen)
	native.max_datagram_size = C.uint32_t(config.MaxDatagramSize)
	native.max_event_bytes = C.uint64_t(config.MaxEventBytes)
	native.max_runtime_queued_bytes = C.uint64_t(config.MaxRuntimeQueuedBytes)
	native.max_session_queued_bytes = C.uint64_t(config.MaxSessionQueuedBytes)
	native.max_endpoints = C.uint32_t(config.MaxEndpoints)
	native.max_pending_handshakes = C.uint32_t(config.MaxPendingHandshakes)
	native.max_sessions_per_endpoint = C.uint32_t(config.MaxSessionsPerEndpoint)
	native.max_sessions_per_ip = C.uint32_t(config.MaxSessionsPerIP)
	native.handshake_rate_per_ip = C.uint32_t(config.HandshakeRatePerIP)
	native.handshake_burst_per_ip = C.uint32_t(config.HandshakeBurstPerIP)
	native.ipv6_admission_prefix_bits = C.uint32_t(config.IPv6AdmissionPrefixBits)
	native.handshake_timeout_ms = C.uint64_t(max(config.HandshakeTimeout.Milliseconds(), 0))
	native.connect_timeout_ms = C.uint64_t(max(config.ConnectTimeout.Milliseconds(), 0))
	native.dns_timeout_ms = C.uint64_t(max(config.DNSTimeout.Milliseconds(), 0))
	if config.TCPNoDelay {
		native.tcp_nodelay = 1
	} else {
		native.tcp_nodelay = 0
	}
	native.tcp_send_buffer_bytes = C.uint64_t(config.TCPSendBufferBytes)
	native.tcp_recv_buffer_bytes = C.uint64_t(config.TCPRecvBufferBytes)
	native.datagram_idle_timeout_ms = C.uint64_t(max(config.DatagramIdleTimeout.Milliseconds(), 0))
	if config.AllowPlaintextBusinessData {
		native.allow_plaintext_business_data = 1
	}
	if config.AllowLegacyEndpoints {
		native.allow_legacy_unauthenticated_endpoints = 1
	}
	native.rekey_after_ms = C.uint64_t(max(config.RekeyAfter.Milliseconds(), 0))
	native.rekey_after_bytes = C.uint64_t(config.RekeyAfterBytes)
	var handle C.rnet_runtime_t
	var loggerHandle cgo.Handle
	if config.Logger != nil {
		loggerHandle = cgo.NewHandle(config.Logger)
	}
	var privateKey *C.uint8_t
	var serverKey *C.uint8_t
	if config.ClientSecurity != nil {
		security := config.ClientSecurity
		if len(security.ExpectedServerPublicKey) != 32 {
			if loggerHandle != 0 {
				loggerHandle.Delete()
			}
			return nil, fmt.Errorf("rnet: expected server public key must contain 32 bytes")
		}
		privateKey = (*C.uint8_t)(unsafe.Pointer(&security.LocalKey.Private[0]))
		serverKey = bytePointer(security.ExpectedServerPublicKey)
	}
	createStatus := C.rnet_go_runtime_create_v5_full(
		&native, privateKey, serverKey, C.uintptr_t(loggerHandle),
		C.uint32_t(config.MinLogLevel), &handle,
	)
	runtime.KeepAlive(config.ClientSecurity)
	if err := statusError(createStatus); err != nil {
		if loggerHandle != 0 {
			loggerHandle.Delete()
		}
		return nil, err
	}
	return &Runtime{handle: handle, logger: loggerHandle}, nil
}

func (r *Runtime) Stop(timeout time.Duration) error {
	defer lockNativeThread()()
	r.mu.Lock()
	if r.handle == 0 || r.stopped {
		r.mu.Unlock()
		return nil
	}
	handle := r.handle
	r.mu.Unlock()
	milliseconds := timeout.Milliseconds()
	if milliseconds < 0 {
		milliseconds = 0
	} else if milliseconds > int64(^uint32(0)) {
		milliseconds = int64(^uint32(0))
	}
	if err := statusError(C.rnet_runtime_stop(handle, C.uint32_t(milliseconds))); err != nil {
		return err
	}
	r.mu.Lock()
	r.stopped = true
	r.mu.Unlock()
	return nil
}

func (r *Runtime) Close() error {
	defer lockNativeThread()()
	if err := r.Stop(0); err != nil {
		return err
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.handle == 0 {
		return nil
	}
	if err := statusError(C.rnet_runtime_destroy(r.handle)); err != nil {
		return err
	}
	r.handle = 0
	if r.logger != 0 {
		r.logger.Delete()
		r.logger = 0
	}
	return nil
}

func (r *Runtime) handleValue() (C.rnet_runtime_t, error) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	if r.handle == 0 {
		return 0, StatusError{Code: int32(C.RNET_E_INVALID_HANDLE), Message: "runtime is closed"}
	}
	return r.handle, nil
}
