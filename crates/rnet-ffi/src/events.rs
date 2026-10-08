use crate::abi::RnetEvent;
use crate::observe::{log_event, maybe_log_latency};
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::runtime_entry;
use std::mem::size_of;
use std::time::Duration;

#[no_mangle]
/// Polls events while returning API status separately from the event count.
///
/// # Safety
/// `out_count` must point to writable memory. For nonzero capacity, `events` must point to
/// writable storage for `capacity` events. Release each nonzero token exactly once before destroy.
pub unsafe extern "C" fn rnet_poll_events(
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
            // Buffer tokens keep data alive until explicitly released by the caller.
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
                correlation_id: event.request_id,
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
