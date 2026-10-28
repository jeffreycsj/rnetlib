//go:build linux && rnet_test_faults

package rnet

/*
#cgo LDFLAGS: -Wl,--wrap=rnet_poll_events -Wl,--wrap=rnet_buffer_release
#include "native.h"
#include <stdatomic.h>
#include <string.h>

// Synthetic ABI buffers exercise the Go ownership contract without allocating
// multi-GiB payloads. Only the explicitly armed runtime is intercepted.
static _Atomic uint64_t transport_fault_runtime;
static size_t transport_fault_length;
static int transport_fault_null, transport_fault_poll, transport_fault_release;
static unsigned transport_released[4];
static const uint8_t transport_bytes[] = {65, 0, 255};

int32_t __real_rnet_poll_events(uint64_t, rnet_event_t *, size_t, uint32_t, size_t *);
int32_t __real_rnet_buffer_release(uint64_t, uint64_t);
int32_t __wrap_rnet_poll_events(uint64_t runtime, rnet_event_t *events,
                               size_t capacity, uint32_t timeout, size_t *count) {
    if (atomic_load(&transport_fault_runtime) != runtime)
        return __real_rnet_poll_events(runtime, events, capacity, timeout, count);
    *count = 0;
    if (capacity < 4) return RNET_E_INVALID_ARGUMENT;
    memset(events, 0, capacity * sizeof(*events));
    for (size_t i = 0; i < 4; ++i) {
        events[i].buffer_token = i + 1;
        events[i].data = transport_bytes;
        events[i].data_len = sizeof(transport_bytes);
    }
    events[1].data = transport_fault_null ? NULL : transport_bytes;
    events[1].data_len = transport_fault_length;
    // The fourth token is not exported, even though its slot has been written.
    *count = 3;
    if (transport_fault_poll) {
        uint16_t port;
        return rnet_endpoint_local_port(runtime, 0, &port);
    }
    return RNET_OK;
}
int32_t __wrap_rnet_buffer_release(uint64_t runtime, uint64_t token) {
    if (atomic_load(&transport_fault_runtime) != runtime)
        return __real_rnet_buffer_release(runtime, token);
    if (token < 1 || token > 4) return RNET_E_INVALID_ARGUMENT;
    ++transport_released[token - 1];
    if (transport_fault_release && token == 1) {
        uint16_t port;
        return rnet_endpoint_local_port(runtime, 0, &port);
    }
    // A real successful ABI call clears the thread-local error, as release does.
    rnet_config_t config;
    return rnet_config_init(&config);
}
static void arm_transport_fault(uint64_t runtime, size_t length, int null_data,
                                int poll_error, int release_error) {
    transport_fault_length = length;
    transport_fault_null = null_data;
    transport_fault_poll = poll_error;
    transport_fault_release = release_error;
    memset(transport_released, 0, sizeof(transport_released));
    atomic_store(&transport_fault_runtime, runtime);
}
static unsigned transport_release_count(size_t index) { return transport_released[index]; }
static void disarm_transport_fault(void) { atomic_store(&transport_fault_runtime, 0); }
*/
import "C"

func armTransportPollFault(r *Runtime, length uint64, nullData, pollError, releaseError bool) {
	flag := func(value bool) C.int {
		if value {
			return 1
		}
		return 0
	}
	handle, _ := r.handleValue()
	C.arm_transport_fault(C.uint64_t(handle), C.size_t(length), flag(nullData), flag(pollError), flag(releaseError))
}

func transportReleaseCounts() [4]uint32 {
	var result [4]uint32
	for i := range result {
		result[i] = uint32(C.transport_release_count(C.size_t(i)))
	}
	return result
}

func disarmTransportPollFault() { C.disarm_transport_fault() }
