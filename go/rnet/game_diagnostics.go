package rnet

/*
#include "native.h"
*/
import "C"

import "time"

// SetMetricsLogInterval controls cumulative P95/P99 summaries sent to the configured
// logger by Poll. Zero disables summaries; the default is 30 seconds.
func (r *GameRuntime) SetMetricsLogInterval(interval time.Duration) error {
	defer lockNativeThread()()
	if interval < 0 {
		return StatusError{Code: int32(C.RNET_E_INVALID_ARGUMENT), Message: "log interval must be nonnegative"}
	}
	handle, err := r.handleValue()
	if err != nil {
		return err
	}
	ms := interval / time.Millisecond
	if interval%time.Millisecond != 0 {
		ms++
	}
	return statusError(C.rnet_game_metrics_log_interval_set(handle, C.uint64_t(ms)))
}
