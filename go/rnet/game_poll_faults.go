//go:build linux && rnet_test_faults

package rnet

/*
#cgo LDFLAGS: -Wl,--wrap=rnet_game_poll_events
#include "native.h"
#include <stdatomic.h>
#include <string.h>

// Test-only linker interception. Buffers are allocated by the real library, so
// its destroy precondition, not a mocked release counter, detects ownership leaks.
static _Atomic uint64_t fault_runtime;
static uint64_t fault_tokens[3];

int32_t __real_rnet_game_poll_events(rnet_runtime_t, rnet_game_event_t *, size_t,
                                    uint32_t, size_t *);
int32_t __wrap_rnet_game_poll_events(rnet_runtime_t runtime, rnet_game_event_t *events,
                                    size_t capacity, uint32_t timeout, size_t *count) {
    uint64_t expected = runtime;
    if (!atomic_compare_exchange_strong(&fault_runtime, &expected, 0))
        return __real_rnet_game_poll_events(runtime, events, capacity, timeout, count);
    *count = 0;
    if (capacity >= 2) {
        rnet_game_buffer_t buffers[3] = {0};
        for (size_t i = 0; i < 3; ++i) {
            int32_t status = rnet_game_prometheus_snapshot(runtime, &buffers[i]);
            if (status != RNET_OK) return status;
            fault_tokens[i] = buffers[i].token;
        }
        memset(events, 0, capacity * sizeof(*events));
        events[0].event_type = RNET_GAME_RESUME_REQUEST;
        events[0].data = buffers[0].data;
        events[0].data_len = buffers[0].len;
        events[0].buffer_token = buffers[0].token;
        events[0].aux_data = buffers[1].data;
        events[0].aux_data_len = buffers[1].len;
        events[0].aux_buffer_token = buffers[1].token;
        // This slot is outside out_count: its token remains test-owned.
        events[1].buffer_token = buffers[2].token;
        *count = 1;
    }
    uint32_t version;
    // Obtain a real thread-local diagnostic; successful releases will clear it.
    return rnet_game_selected_protocol_version(runtime, 0, &version);
}
static void arm_poll_failure(uint64_t runtime) {
    memset(fault_tokens, 0, sizeof(fault_tokens));
    atomic_store(&fault_runtime, runtime);
}
static int32_t release_unexported(uint64_t runtime) {
    return rnet_game_buffer_release(runtime, fault_tokens[2]);
}
static void cleanup_poll_failure(uint64_t runtime) {
    atomic_store(&fault_runtime, 0);
    for (size_t i = 0; i < 3; ++i)
        if (fault_tokens[i] != 0) (void)rnet_game_buffer_release(runtime, fault_tokens[i]);
}
*/
import "C"

func armGamePollFailure(r *GameRuntime) {
	handle, _ := r.handleValue()
	C.arm_poll_failure(C.uint64_t(handle))
}

func releaseUnexportedGameBuffer(r *GameRuntime) int32 {
	handle, _ := r.handleValue()
	return int32(C.release_unexported(C.uint64_t(handle)))
}

func cleanupGamePollFailure(r *GameRuntime) {
	handle, _ := r.handleValue()
	C.cleanup_poll_failure(C.uint64_t(handle))
}
