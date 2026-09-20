use crate::abi::RnetLatencyMetric;
use crate::abi::RnetLogger;
use crate::abi::RnetLoggerV2;
use crate::abi::RnetMetrics;
use crate::abi::RNET_ABI_VERSION;
use crate::abi::{RnetLatencyMetricV2, RnetMetricsV2, RnetMetricsV3};
use crate::registry::ffi_status;
use crate::registry::invalid_argument;
use crate::registry::runtime_entry;
use crate::registry::validate_struct;
use crate::registry::LogCallbackGuard;
use crate::registry::RuntimeEntry;
use crate::registry::IN_LOG_CALLBACK;
use rnet_core::ErrorCode;
use rnet_core::Result;
use rnet_core::RnetError;
use rnet_core::{Event, EventType};
use rnet_observe::BoundedLogger;
use rnet_observe::LogLevel;
use rnet_observe::LogRecord;
use rnet_observe::LoggerConfig;
use rnet_transport::LatencyKind;
use rnet_transport::MetricsSnapshot;
use rnet_transport::LATENCY_KIND_COUNT;
use std::ffi::c_void;
use std::mem::size_of;
use std::str;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

#[no_mangle]
/// Sets the interval for aggregated latency summary logs. Zero disables summaries.
pub extern "C" fn rnet_metrics_log_interval_set(runtime: u64, interval_ms: u64) -> i32 {
    ffi_status(|| {
        let entry = runtime_entry(runtime)?;
        entry
            .metrics_log_interval_ms
            .store(interval_ms, Ordering::Relaxed);
        *entry
            .last_metrics_log
            .lock()
            .expect("metrics log timestamp poisoned") = Instant::now();
        Ok(())
    })
}

#[no_mangle]
/// Copies the current metrics counters into caller-owned memory.
///
/// # Safety
/// `out` must point to writable memory for one `RnetMetrics`.
pub unsafe extern "C" fn rnet_metrics_snapshot(runtime: u64, out: *mut RnetMetrics) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("metrics output is null");
        }
        let entry = runtime_entry(runtime)?;
        let MetricsSnapshot {
            frames_received,
            frames_sent,
            bytes_received,
            bytes_sent,
            events_dropped,
            send_would_block,
            protocol_errors,
            ..
        } = entry.network.metrics_snapshot();
        let logger = entry.logger.lock().expect("logger lock poisoned");
        let (logs_dropped, logger_panics) = logger
            .as_ref()
            .map(|logger| (logger.dropped(), logger.sink_panics()))
            .unwrap_or_default();
        unsafe {
            out.write(RnetMetrics {
                struct_size: size_of::<RnetMetrics>() as u32,
                abi_version: RNET_ABI_VERSION,
                frames_received,
                frames_sent,
                bytes_received,
                bytes_sent,
                events_dropped,
                send_would_block,
                protocol_errors,
                logs_dropped,
                logger_panics,
            })
        };
        Ok(())
    })
}

#[no_mangle]
/// Copies production counters, gauges, and close-reason series into caller-owned memory.
///
/// # Safety
/// `out` must point to writable memory for one `RnetMetricsV2`.
pub unsafe extern "C" fn rnet_metrics_snapshot_v2(runtime: u64, out: *mut RnetMetricsV2) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("metrics output is null");
        }
        let entry = runtime_entry(runtime)?;
        let metrics = entry.network.metrics_snapshot();
        let logger = entry.logger.lock().expect("logger lock poisoned");
        let (logs_dropped, logger_panics) = logger
            .as_ref()
            .map(|logger| (logger.dropped(), logger.sink_panics()))
            .unwrap_or_default();
        unsafe {
            out.write(RnetMetricsV2 {
                struct_size: size_of::<RnetMetricsV2>() as u32,
                abi_version: RNET_ABI_VERSION,
                frames_received: metrics.frames_received,
                frames_sent: metrics.frames_sent,
                bytes_received: metrics.bytes_received,
                bytes_sent: metrics.bytes_sent,
                events_dropped: metrics.events_dropped,
                send_would_block: metrics.send_would_block,
                protocol_errors: metrics.protocol_errors,
                lifecycle_events_rejected: metrics.lifecycle_events_rejected,
                admission_rejected: metrics.admission_rejected,
                queued_send_bytes: metrics.queued_send_bytes,
                peak_queued_send_bytes: metrics.peak_queued_send_bytes,
                queued_event_bytes: metrics.queued_event_bytes,
                session_closed_by_reason: metrics.session_closed_by_reason,
                logs_dropped,
                logger_panics,
            })
        };
        Ok(())
    })
}

#[no_mangle]
/// Copies production counters and runtime resource gauges into caller-owned memory.
///
/// # Safety
/// `out` must point to writable memory for one `RnetMetricsV3`.
pub unsafe extern "C" fn rnet_metrics_snapshot_v3(runtime: u64, out: *mut RnetMetricsV3) -> i32 {
    ffi_status(|| {
        if out.is_null() {
            return invalid_argument("metrics output is null");
        }
        let entry = runtime_entry(runtime)?;
        let metrics = entry.network.metrics_snapshot();
        let logger = entry.logger.lock().expect("logger lock poisoned");
        let (logs_dropped, logger_panics) = logger
            .as_ref()
            .map(|logger| (logger.dropped(), logger.sink_panics()))
            .unwrap_or_default();
        unsafe {
            out.write(RnetMetricsV3 {
                struct_size: size_of::<RnetMetricsV3>() as u32,
                abi_version: RNET_ABI_VERSION,
                frames_received: metrics.frames_received,
                frames_sent: metrics.frames_sent,
                bytes_received: metrics.bytes_received,
                bytes_sent: metrics.bytes_sent,
                events_dropped: metrics.events_dropped,
                send_would_block: metrics.send_would_block,
                protocol_errors: metrics.protocol_errors,
                lifecycle_events_rejected: metrics.lifecycle_events_rejected,
                admission_rejected: metrics.admission_rejected,
                queued_send_bytes: metrics.queued_send_bytes,
                peak_queued_send_bytes: metrics.peak_queued_send_bytes,
                queued_event_bytes: metrics.queued_event_bytes,
                session_closed_by_reason: metrics.session_closed_by_reason,
                logs_dropped,
                logger_panics,
                current_endpoints: metrics.current_endpoints,
                current_sessions: metrics.current_sessions,
                established_sessions: metrics.established_sessions,
                pending_handshakes: metrics.pending_handshakes,
                peak_pending_handshakes: metrics.peak_pending_handshakes,
                admission_rejected_by_reason: metrics.admission_rejected_by_reason,
            })
        };
        Ok(())
    })
}

#[no_mangle]
/// Copies all latency percentile snapshots into caller-owned memory.
///
/// A zero-capacity call may use a null `metrics` pointer to query the required count.
///
/// # Safety
/// `out_count` must point to writable memory. For nonzero capacity, `metrics` must point to
/// writable storage for at least `capacity` entries.
pub unsafe extern "C" fn rnet_latency_snapshot(
    runtime: u64,
    metrics: *mut RnetLatencyMetric,
    capacity: usize,
    out_count: *mut usize,
) -> i32 {
    ffi_status(|| {
        if out_count.is_null() {
            return invalid_argument("latency metric count output is null");
        }
        let entry = runtime_entry(runtime)?;
        let snapshots = latency_snapshots(&entry);
        unsafe { out_count.write(LATENCY_KIND_COUNT) };
        if capacity == 0 {
            return Ok(());
        }
        if metrics.is_null() {
            return invalid_argument("latency metrics output is null");
        }
        if capacity < LATENCY_KIND_COUNT {
            return invalid_argument("latency metrics capacity is too small");
        }
        for (index, metric) in snapshots.into_iter().enumerate() {
            let latency = metric.latency;
            unsafe {
                metrics.add(index).write(RnetLatencyMetric {
                    struct_size: size_of::<RnetLatencyMetric>() as u32,
                    kind: metric.kind as u32,
                    sample_count: latency.sample_count,
                    p50_us: latency.p50_us,
                    p90_us: latency.p90_us,
                    p95_us: latency.p95_us,
                    p99_us: latency.p99_us,
                    max_us: latency.max_us,
                })
            };
        }
        Ok(())
    })
}

#[no_mangle]
/// Copies latency snapshots including P99.9. Set `drain_window` to rotate the interval.
///
/// # Safety
/// `out_count` and each requested output entry must point to writable caller-owned memory.
pub unsafe extern "C" fn rnet_latency_snapshot_v2(
    runtime: u64,
    metrics: *mut RnetLatencyMetricV2,
    capacity: usize,
    out_count: *mut usize,
    drain_window: u32,
) -> i32 {
    ffi_status(|| {
        if out_count.is_null() {
            return invalid_argument("latency metric count output is null");
        }
        let entry = runtime_entry(runtime)?;
        unsafe { out_count.write(LATENCY_KIND_COUNT) };
        if capacity == 0 {
            return Ok(());
        }
        if metrics.is_null() || capacity < LATENCY_KIND_COUNT {
            return invalid_argument("latency metrics output is null or too small");
        }
        let snapshots = if drain_window == 0 {
            entry.network.latency_snapshot()
        } else {
            entry.network.drain_latency_window()
        };
        for (index, metric) in snapshots.into_iter().enumerate() {
            let latency = metric.latency;
            unsafe {
                metrics.add(index).write(RnetLatencyMetricV2 {
                    struct_size: size_of::<RnetLatencyMetricV2>() as u32,
                    kind: metric.kind as u32,
                    sample_count: latency.sample_count,
                    p50_us: latency.p50_us,
                    p90_us: latency.p90_us,
                    p95_us: latency.p95_us,
                    p99_us: latency.p99_us,
                    p999_us: latency.p999_us,
                    max_us: latency.max_us,
                })
            };
        }
        Ok(())
    })
}

pub(crate) unsafe fn build_logger(logger: *const RnetLogger) -> Result<Option<BoundedLogger>> {
    if logger.is_null() {
        return Ok(None);
    }
    let logger = unsafe { *logger };
    validate_struct(
        logger.struct_size,
        logger.abi_version,
        size_of::<RnetLogger>(),
    )?;
    let callback = logger
        .log
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidArgument, "logger callback is null"))?;
    let user_data = logger.user_data as usize;
    let min_level = logger.min_level.min(LogLevel::Error as u32);
    BoundedLogger::new(LoggerConfig::default(), move |record| {
        if (record.level as u32) < min_level {
            return;
        }
        IN_LOG_CALLBACK.with(|flag| flag.set(true));
        let _callback_guard = LogCallbackGuard;
        unsafe {
            callback(
                user_data as *mut c_void,
                record.level as u32,
                record.target.as_ptr(),
                record.target.len(),
                record.message.as_ptr(),
                record.message.len(),
            )
        };
    })
    .map(Some)
}

pub(crate) unsafe fn build_logger_v2(logger: *const RnetLoggerV2) -> Result<Option<BoundedLogger>> {
    if logger.is_null() {
        return Ok(None);
    }
    let logger = unsafe { *logger };
    validate_struct(
        logger.struct_size,
        logger.abi_version,
        size_of::<RnetLoggerV2>(),
    )?;
    let callback = logger
        .log
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidArgument, "logger callback is null"))?;
    let user_data = logger.user_data as usize;
    let min_level = logger.min_level.min(LogLevel::Error as u32);
    BoundedLogger::new(LoggerConfig::default(), move |record| {
        if (record.level as u32) < min_level {
            return;
        }
        IN_LOG_CALLBACK.with(|flag| flag.set(true));
        let _callback_guard = LogCallbackGuard;
        unsafe {
            callback(
                user_data as *mut c_void,
                record.timestamp_unix_ms,
                record.level as u32,
                record.event_name.as_ptr(),
                record.event_name.len(),
                record.runtime,
                record.endpoint,
                record.session,
                record.transport,
                record.error_code as i32,
                record.correlation_id,
                record.message.as_ptr(),
                record.message.len(),
            )
        };
    })
    .map(Some)
}

pub(crate) fn log_entry(
    entry: &RuntimeEntry,
    runtime: u64,
    event_name: &str,
    level: LogLevel,
    message: &str,
) {
    if let Some(logger) = entry.logger.lock().expect("logger lock poisoned").as_ref() {
        logger.log(LogRecord {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            level,
            event_name: event_name.into(),
            target: "rnet.runtime".into(),
            message: message.into(),
            runtime,
            endpoint: 0,
            session: 0,
            transport: 0,
            error_code: ErrorCode::Ok,
            correlation_id: 0,
        });
    }
}

pub(crate) fn log_event(entry: &RuntimeEntry, runtime: u64, event: &Event) {
    let (event_name, level) = match event.event_type {
        EventType::RuntimeStarted => ("runtime_started", LogLevel::Info),
        EventType::EndpointOpened => ("endpoint_opened", LogLevel::Info),
        EventType::EndpointError => ("endpoint_error", LogLevel::Error),
        EventType::SessionOpened => ("session_opened", LogLevel::Info),
        EventType::SessionClosed if event.status == ErrorCode::Ok => {
            ("session_closed", LogLevel::Info)
        }
        EventType::SessionClosed => ("session_closed", LogLevel::Warn),
        EventType::RuntimeStopped => ("runtime_stopped", LogLevel::Info),
        EventType::AuthRequest => ("auth_requested", LogLevel::Info),
        EventType::JoinFailed => ("join_failed", LogLevel::Error),
        EventType::SecurityChanged => ("security_changed", LogLevel::Info),
        EventType::Message | EventType::Writable | EventType::GameControl => return,
    };
    let transport = if event.endpoint == 0 {
        0
    } else {
        entry
            .network
            .endpoint_transport(event.endpoint)
            .map_or(0, |transport| transport as u32)
    };
    if let Some(logger) = entry.logger.lock().expect("logger lock poisoned").as_ref() {
        let message = if matches!(
            event.event_type,
            EventType::EndpointError | EventType::JoinFailed
        ) {
            let detail = String::from_utf8_lossy(&event.data);
            format!(
                "{event_name} status={} detail={:.512}",
                event.status as i32, detail
            )
        } else {
            format!("{event_name} status={}", event.status as i32)
        };
        logger.log(LogRecord {
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            level,
            event_name: event_name.into(),
            target: "rnet.event".into(),
            message,
            runtime,
            endpoint: event.endpoint,
            session: event.session,
            transport,
            error_code: event.status,
            correlation_id: event.request_id,
        });
    }
}

pub(crate) fn maybe_log_latency(entry: &RuntimeEntry, runtime: u64) {
    let interval_ms = entry.metrics_log_interval_ms.load(Ordering::Relaxed);
    if interval_ms == 0 {
        return;
    }
    let now = Instant::now();
    let mut last = entry
        .last_metrics_log
        .lock()
        .expect("metrics log timestamp poisoned");
    if now.saturating_duration_since(*last) < Duration::from_millis(interval_ms) {
        return;
    }
    *last = now;
    drop(last);

    let mut message = String::from("latency_summary");
    for metric in latency_snapshots(entry) {
        let latency = metric.latency;
        use std::fmt::Write as _;
        let _ = write!(
            message,
            " kind={} count={} p50_us={} p90_us={} p95_us={} p99_us={} p999_us={} max_us={}",
            metric.kind as u32,
            latency.sample_count,
            latency.p50_us,
            latency.p90_us,
            latency.p95_us,
            latency.p99_us,
            latency.p999_us,
            latency.max_us,
        );
    }
    log_entry(entry, runtime, "latency_summary", LogLevel::Info, &message);
}

fn latency_snapshots(
    entry: &RuntimeEntry,
) -> [rnet_transport::LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
    let mut snapshots = entry.network.latency_snapshot();
    if let Some(logger) = entry.logger.lock().expect("logger lock poisoned").as_ref() {
        snapshots[LatencyKind::LoggerCallback as usize - 1].latency = logger.callback_latency();
    }
    snapshots
}
