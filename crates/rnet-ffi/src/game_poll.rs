//! Event polling and game-runtime lifecycle entry points.

use crate::game_abi::{RnetGameEvent, RnetGameEventV2};
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
/// # Safety
/// `events` must hold `capacity` writable V2 events; `out_count` must be writable. Each nonzero
/// token in the nested base event must be released exactly once before runtime destroy.
pub unsafe extern "C" fn rnet_game_poll_events_v2(
    runtime: u64,
    events: *mut RnetGameEventV2,
    capacity: usize,
    timeout_ms: u32,
    out_count: *mut usize,
) -> i32 {
    ffi_status(|| {
        if out_count.is_null() || (capacity != 0 && events.is_null()) {
            return invalid_argument("invalid game V2 event output");
        }
        unsafe { out_count.write(0) };
        let entry = game_registry::lease(runtime)?;
        if capacity == 0 {
            let _ = entry.runtime.poll(0, Duration::ZERO);
            return Ok(());
        }
        let polled = entry
            .runtime
            .poll(capacity, Duration::from_millis(u64::from(timeout_ms)));
        for (index, event) in polled.into_iter().enumerate() {
            let converted = game_events::encode_v2(event, &entry.buffers);
            unsafe { events.add(index).write(converted) };
            unsafe { out_count.write(index + 1) };
        }
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_game_buffer_release(runtime: u64, token: u64) -> i32 {
    ffi_status(|| game_registry::lease(runtime)?.buffers.release(token))
}

#[no_mangle]
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
pub extern "C" fn rnet_game_runtime_destroy(runtime: u64) -> i32 {
    ffi_status(|| {
        if IN_LOG_CALLBACK.with(Cell::get) {
            return invalid_state("cannot destroy a game runtime from a logger callback");
        }
        game_registry::destroy(runtime)
    })
}
