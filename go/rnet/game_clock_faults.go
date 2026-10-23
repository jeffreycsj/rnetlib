//go:build linux && rnet_test_faults

package rnet

/*
#cgo LDFLAGS: -Wl,--wrap=rnet_game_clock_sync_snapshot
#include "native.h"
#include <stdatomic.h>
#include <string.h>

// One-shot test-only ABI boundary injection, never compiled into ordinary SDKs.
// Exercise the actual Go snapshot conversion without waiting centuries for a
// local clock origin. Non-target calls still use the real native implementation.
static _Atomic uint64_t clock_fault_runtime;
static int64_t clock_fault_offset;
static uint32_t clock_fault_available;
int32_t __real_rnet_game_clock_sync_snapshot(rnet_runtime_t, rnet_session_t,
                                            rnet_game_clock_sync_t *);
int32_t __wrap_rnet_game_clock_sync_snapshot(rnet_runtime_t runtime,
        rnet_session_t session, rnet_game_clock_sync_t *out) {
    uint64_t expected = runtime;
    if (!runtime || !atomic_compare_exchange_strong(&clock_fault_runtime, &expected, 0))
        return __real_rnet_game_clock_sync_snapshot(runtime, session, out);
    memset(out, 0, sizeof(*out));
    out->struct_size = sizeof(*out);
    out->abi_version = RNET_ABI_VERSION;
    out->available = clock_fault_available;
    out->server_minus_client_us = clock_fault_offset;
    if (out->available) { out->rtt_us = 25; out->samples = 3; }
    return RNET_OK;
}
static void arm_clock_snapshot(uint64_t runtime, int64_t offset, uint32_t available) {
    clock_fault_offset = offset;
    clock_fault_available = available;
    atomic_store(&clock_fault_runtime, runtime);
}
static void clear_clock_snapshot(void) { atomic_store(&clock_fault_runtime, 0); }
*/
import "C"

func armGameClockSnapshot(r *GameRuntime, offset int64, available bool) {
	handle, _ := r.handleValue()
	var flag C.uint32_t
	if available {
		flag = 1
	}
	C.arm_clock_snapshot(C.uint64_t(handle), C.int64_t(offset), flag)
}

func clearGameClockSnapshot() { C.clear_clock_snapshot() }
