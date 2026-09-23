package rnet

/*
#include "native.h"
*/
import "C"

import "fmt"

// gameNetworkConfig preserves the existing transport tuning contract while
// keeping game credentials and logging separate from the low-level runtime.
func gameNetworkConfig(config Config) (C.rnet_config_v5_t, error) {
	defer lockNativeThread()()
	var native C.rnet_config_v5_t
	if err := statusError(C.rnet_config_v5_init(&native)); err != nil {
		return native, err
	}
	if config.Logger != nil || config.ClientSecurity != nil {
		return native, fmt.Errorf("rnet: game network config cannot carry a transport logger or client identity")
	}
	if config.AllowLegacyEndpoints {
		return native, fmt.Errorf("rnet: game runtime forbids legacy unauthenticated endpoints")
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
	native.datagram_idle_timeout_ms = C.uint64_t(max(config.DatagramIdleTimeout.Milliseconds(), 0))
	native.rekey_after_ms = C.uint64_t(max(config.RekeyAfter.Milliseconds(), 0))
	native.rekey_after_bytes = C.uint64_t(config.RekeyAfterBytes)
	if config.TCPNoDelay {
		native.tcp_nodelay = 1
	} else {
		native.tcp_nodelay = 0
	}
	native.tcp_send_buffer_bytes = C.uint64_t(config.TCPSendBufferBytes)
	native.tcp_recv_buffer_bytes = C.uint64_t(config.TCPRecvBufferBytes)
	// RnetGameConfig.AllowPlaintextBusinessData is the only policy switch.
	native.allow_plaintext_business_data = 0
	native.allow_legacy_unauthenticated_endpoints = 0
	return native, nil
}
