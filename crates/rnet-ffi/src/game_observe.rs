//! Low-cardinality game telemetry entry points.

use crate::game_abi::{RnetGameBuffer, RnetGameClockSync, RnetGameMetrics, RnetGameQuality};
use crate::game_queue_abi::{RnetGameRealtimeQueue, RnetGameScheduledQueue};
use crate::game_registry;
use crate::registry::{ffi_status, invalid_argument};

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one `RnetGameQuality`.
pub unsafe extern "C" fn rnet_game_network_quality(
    runtime: u64,
    session: u64,
    out: *mut RnetGameQuality,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game quality output is null");
        }
        let quality = game_registry::lease(runtime)?
            .runtime
            .network_quality(session)?;
        unsafe { out.write(quality.map_or_else(RnetGameQuality::default, Into::into)) };
        Ok(())
    })
}

#[no_mangle]
/// Runtime-local monotonic microseconds, not a wall-clock timestamp.
/// # Safety
/// `out` must point to writable storage for one `u64`.
pub unsafe extern "C" fn rnet_game_clock_micros(runtime: u64, out: *mut u64) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game clock output is null");
        }
        let now = game_registry::lease(runtime)?.runtime.clock_micros();
        unsafe { out.write(now) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one `RnetGameClockSync`.
pub unsafe extern "C" fn rnet_game_clock_sync_snapshot(
    runtime: u64,
    session: u64,
    out: *mut RnetGameClockSync,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game clock snapshot output is null");
        }
        let sample = game_registry::lease(runtime)?
            .runtime
            .clock_sync_snapshot(session)?;
        unsafe { out.write(sample.map_or_else(RnetGameClockSync::default, Into::into)) };
        Ok(())
    })
}

#[no_mangle]
/// Returns cumulative, low-cardinality game counters. Snapshots of concurrent counters are
/// not globally atomic; `logger_available` distinguishes absent logging from zero drops.
/// # Safety
/// `out` must point to writable storage for one `RnetGameMetrics`.
pub unsafe extern "C" fn rnet_game_metrics_snapshot(
    runtime: u64,
    out: *mut RnetGameMetrics,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game metrics output is null");
        }
        let entry = game_registry::lease(runtime)?;
        unsafe { out.write(RnetGameMetrics::from_runtime(&entry.runtime)) };
        Ok(())
    })
}

#[no_mangle]
/// Returns runtime-wide `LatestOnly` staging gauges and cumulative loss counters.
///
/// # Safety
/// `out` must point to writable storage for one `RnetGameRealtimeQueue`.
pub unsafe extern "C" fn rnet_game_realtime_queue_snapshot(
    runtime: u64,
    out: *mut RnetGameRealtimeQueue,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game realtime queue output is null");
        }
        let snapshot = game_registry::lease(runtime)?
            .runtime
            .realtime_queue_snapshot();
        unsafe { out.write(snapshot.into()) };
        Ok(())
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one `RnetGameScheduledQueue`.
pub unsafe extern "C" fn rnet_game_scheduled_queue_snapshot(
    runtime: u64,
    out: *mut RnetGameScheduledQueue,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game scheduled queue output is null");
        }
        let snapshot = game_registry::lease(runtime)?
            .runtime
            .scheduled_queue_snapshot();
        unsafe { out.write(snapshot.into()) };
        Ok(())
    })
}

#[no_mangle]
/// Returns a borrowed, token-owned Prometheus text snapshot without player labels.
///
/// # Safety
/// `out` must point to writable storage for one `RnetGameBuffer`. Release a nonzero token with
/// `rnet_game_buffer_release` after the last read; the pointer is invalid afterward.
pub unsafe extern "C" fn rnet_game_prometheus_snapshot(
    runtime: u64,
    out: *mut RnetGameBuffer,
) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("game Prometheus output is null");
        }
        let entry = game_registry::lease(runtime)?;
        let mut buffer = RnetGameBuffer::default();
        if let Some(view) = entry
            .buffers
            .insert(entry.runtime.prometheus_snapshot().into_bytes())
        {
            buffer.data = view.ptr;
            buffer.len = view.len;
            buffer.token = view.token;
        }
        unsafe { out.write(buffer) };
        Ok(())
    })
}
