use crate::abi::RnetEvent;
use crate::observe::{log_event, maybe_log_latency};
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::runtime_entry;
use std::mem::size_of;
use std::panic::catch_unwind;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

#[no_mangle]
/// Polls up to `capacity` events into caller-owned memory.
///
/// # Safety
/// For nonzero capacity, `events` must point to writable storage for `capacity` events. Returned
/// data pointers must not be read after their buffer token is released.
pub unsafe extern "C" fn rnet_poll_events(
    runtime: u64,
    events: *mut RnetEvent,
    capacity: usize,
    timeout_ms: u32,
) -> usize {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if capacity == 0 {
            return Ok(0);
        }
        if events.is_null() {
            return invalid_argument("events is null");
        }
        let entry = runtime_entry(runtime)?;
        let polled = entry
            .network
            .poll_events(capacity, Duration::from_millis(u64::from(timeout_ms)));
        for (index, event) in polled.iter().enumerate() {
            log_event(&entry, runtime, event);
            // The polled Rust event is temporary. FFI callers receive a stable buffer token and
            // must release it exactly once after consuming the pointer; the runtime stays alive
            // until all outstanding tokens have been returned.
            let view = if event.data.is_empty() {
                None
            } else {
                Some(entry.buffers.insert(event.data.clone()))
            };
            let output = RnetEvent {
                struct_size: size_of::<RnetEvent>() as u32,
                event_type: event.event_type as u32,
                endpoint: event.endpoint,
                session: event.session,
                msg_type: event.msg_type,
                stream_id: event.stream_id,
                request_id: event.request_id,
                data: view.map_or(std::ptr::null(), |buffer| buffer.ptr),
                data_len: view.map_or(0, |buffer| buffer.len),
                buffer_token: view.map_or(0, |buffer| buffer.token),
                status: event.status as i32,
            };
            unsafe { events.add(index).write(output) };
        }
        maybe_log_latency(&entry, runtime);
        Ok(polled.len())
    }));
    match result {
        Ok(Ok(count)) => count,
        Ok(Err(_)) | Err(_) => 0,
    }
}

#[no_mangle]
/// Polls events while returning API status separately from the event count.
///
/// # Safety
/// `out_count` must point to writable memory. For nonzero capacity, `events` must point to
/// writable storage for `capacity` events. Returned buffers follow `rnet_poll_events` ownership.
pub unsafe extern "C" fn rnet_poll_events_ex(
    runtime: u64,
    events: *mut RnetEvent,
    capacity: usize,
    timeout_ms: u32,
    out_count: *mut usize,
) -> i32 {
    ffi_status(|| {
        if out_count.is_null() {
            return invalid_argument("event count output is null");
        }
        unsafe { out_count.write(0) };
        if capacity != 0 && events.is_null() {
            return invalid_argument("events is null for nonzero capacity");
        }
        let entry = runtime_entry(runtime)?;
        maybe_log_latency(&entry, runtime);
        if capacity == 0 {
            return Ok(());
        }
        let polled = entry
            .network
            .poll_events(capacity, Duration::from_millis(u64::from(timeout_ms)));
        for (index, event) in polled.iter().enumerate() {
            log_event(&entry, runtime, event);
            // Keep the v2 status/count API on the same ownership contract as the legacy poll.
            let view = if event.data.is_empty() {
                None
            } else {
                Some(entry.buffers.insert(event.data.clone()))
            };
            let output = RnetEvent {
                struct_size: size_of::<RnetEvent>() as u32,
                event_type: event.event_type as u32,
                endpoint: event.endpoint,
                session: event.session,
                msg_type: event.msg_type,
                stream_id: event.stream_id,
                request_id: event.request_id,
                data: view.map_or(std::ptr::null(), |buffer| buffer.ptr),
                data_len: view.map_or(0, |buffer| buffer.len),
                buffer_token: view.map_or(0, |buffer| buffer.token),
                status: event.status as i32,
            };
            unsafe { events.add(index).write(output) };
        }
        unsafe { out_count.write(polled.len()) };
        Ok(())
    })
}

#[no_mangle]
pub extern "C" fn rnet_buffer_release(runtime: u64, buffer_token: u64) -> i32 {
    ffi_status(|| runtime_entry(runtime)?.buffers.release(buffer_token))
}
