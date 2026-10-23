//! Event polling and game-runtime lifecycle entry points.

use crate::game_abi::RnetGameEvent;
use crate::game_events;
use crate::game_registry;
use crate::registry::{ffi_status, invalid_argument, invalid_state, IN_LOG_CALLBACK};
use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[no_mangle]
/// # Safety
/// `events` must hold `capacity` writable events; `out_count` must be writable. Each nonzero
/// returned buffer token must be released exactly once before runtime destroy.
/// Drives negotiation, heartbeat, clock, queues and logs while exporting up to capacity
/// events into events; required out_count gives the exported prefix, also on failure.
/// Positive capacity requires a full array. timeout_ms bounds idle waiting, not lock/work
/// time; zero is nonblocking. capacity=0 permits NULL events and still drives maintenance
/// without returning application events. Both peers must poll regularly even when idle.
/// Ordinary staged events have separate count/byte limits inherited from the network event
/// queue. At those limits, transport draining pauses but timers continue. Use positive
/// capacity regularly: zero-capacity calls alone can block ACK processing and time out.
/// Release both nonzero tokens of every returned event separately; sensitive native
/// buffers are erased on release. Copies made by the application remain its responsibility.
pub unsafe extern "C" fn rnet_game_poll_events(
    runtime: u64,
    events: *mut RnetGameEvent,
    capacity: usize,
    timeout_ms: u32,
    out_count: *mut usize,
) -> i32 {
    ffi_status(|| {
        if out_count.is_null() || (capacity != 0 && events.is_null()) {
            return invalid_argument("invalid game event output");
        }
        unsafe { out_count.write(0) };
        let entry = game_registry::lease(runtime)?;
        if capacity == 0 {
            // A zero-capacity call is a nonblocking maintenance tick. This keeps authenticated
            // controls and liveness state progressing without requiring a dummy event buffer.
            let _ = entry.runtime.poll(0, Duration::ZERO);
            return Ok(());
        }
        let polled = entry
            .runtime
            .poll(capacity, Duration::from_millis(u64::from(timeout_ms)));
        for (index, event) in polled.into_iter().enumerate() {
            let converted = game_events::encode(event, &entry.buffers);
            unsafe { events.add(index).write(converted) };
            unsafe { out_count.write(index + 1) };
        }
        Ok(())
    })
}

#[no_mangle]
/// Releases one nonzero game event/export token; data becomes invalid immediately.
/// Zero/stale/already released tokens fail in that runtime; foreign runtimes can have
/// coincident token values. Do not mix owners or use transport release here.
pub extern "C" fn rnet_game_buffer_release(runtime: u64, token: u64) -> i32 {
    ffi_status(|| game_registry::lease(runtime)?.buffers.release(token))
}

#[no_mangle]
/// Stops game/transport work with drain_timeout_ms send budget (zero means no drain).
/// If WOULD_BLOCK, poll and retry to publish final events. Keep handle for buffer release;
/// stop/destroy may not be called from logger callbacks.
pub extern "C" fn rnet_game_runtime_stop(runtime: u64, drain_timeout_ms: u32) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("cannot stop a game runtime from a logger callback");
        }
        let entry = game_registry::lease(runtime)?;
        entry
            .runtime
            .stop(Duration::from_millis(u64::from(drain_timeout_ms)))?;
        entry.stopped.store(true, Ordering::Release);
        Ok(())
    })
}

#[no_mangle]
/// Destroys a successfully stopped game runtime after all tokens are released.
/// Active calls yield WOULD_BLOCK; unstopped runtime/outstanding tokens yield INVALID_STATE.
/// Only success invalidates handle; serialize lifetime changes and retry after resolving errors.
pub extern "C" fn rnet_game_runtime_destroy(runtime: u64) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("cannot destroy a game runtime from a logger callback");
        }
        game_registry::destroy(runtime)
    })
}
