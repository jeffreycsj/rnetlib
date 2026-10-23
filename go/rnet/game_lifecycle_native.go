//go:build linux && rnet_test_faults

package rnet

/*
#include "native.h"
*/
import "C"

// Keep an actual ABI-owned buffer outstanding across Close. The public Go
// snapshot API copies and releases it immediately; only this test build exposes
// its token so native destroy, not a mocked return code, enforces the lifetime.
func retainGameLifecycleBuffer(r *GameRuntime) (uint64, uint64, error) {
	defer lockNativeThread()()
	handle, err := r.handleValue()
	if err != nil {
		return 0, 0, err
	}
	var buffer C.rnet_game_buffer_t
	err = statusError(C.rnet_game_prometheus_snapshot(handle, &buffer))
	return uint64(handle), uint64(buffer.token), err
}

func releaseGameLifecycleBuffer(handle, token uint64) error {
	defer lockNativeThread()()
	return statusError(C.rnet_game_buffer_release(C.rnet_runtime_t(handle), C.uint64_t(token)))
}

func stopGameLifecycleHandle(handle uint64) error {
	defer lockNativeThread()()
	return statusError(C.rnet_game_runtime_stop(C.rnet_runtime_t(handle), 0))
}
