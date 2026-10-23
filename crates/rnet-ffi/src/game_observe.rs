//! Low-cardinality game telemetry entry points.

use crate::game_abi::{RnetGameBuffer, RnetGameClockSync, RnetGameMetrics, RnetGameQuality};
use crate::game_queue_abi::{RnetGameRealtimeQueue, RnetGameScheduledQueue};
use crate::game_registry;
use crate::registry::{ffi_status, invalid_argument};

/// Changes poll-driven cumulative game latency logs; zero disables them.
#[no_mangle]
/// Sets poll-driven cumulative game log interval_ms (default 30000); zero disables.
pub extern "C" fn rnet_game_metrics_log_interval_set(runtime: u64, interval_ms: u64) -> i32 {
    ffi_status(|| {
        game_registry::lease(runtime)?
            .runtime
            .set_metrics_log_interval(std::time::Duration::from_millis(interval_ms))
    })
}

#[no_mangle]
/// # Safety
/// `out` must point to writable storage for one `RnetGameQuality`.
/// Writes a session's quality to required out; available=0 means no heartbeat sample.
/// UDP gaps and KCP retransmissions have different meanings; TCP has no loss estimate.
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
/// Writes this runtime's monotonic microseconds to required out; not Unix time.
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
/// Writes approximate client-side server-minus-client offset to required out.
/// session must be game-ready; pending/stale handles return an error, not available=0.
/// available=0 before sampling and on server sessions; not a trusted anti-cheat clock.
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
/// Writes cumulative counters/gauges to required out; no reset or globally atomic snapshot.
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
/// Writes runtime-wide LatestOnly game queue gauges/counters to required out; forwarded is not delivered.
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
/// Writes priority queue gauges, cumulative delays and outcomes to required out; no reset.
/// tick_* delays cover only tick-bearing messages, not a per-poll time window.
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
/// Exports low-cardinality Prometheus text into required out; not NUL-terminated.
/// Release its nonzero token with rnet_game_buffer_release; does not reset counters.
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
