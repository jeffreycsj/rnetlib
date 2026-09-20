use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rnet_core::ErrorCode;
use rnet_observe::{BoundedLogger, LogLevel, LogRecord, LoggerConfig};

fn record(level: LogLevel, message: &str) -> LogRecord {
    LogRecord {
        timestamp_unix_ms: 0,
        level,
        event_name: "test".into(),
        target: "test".into(),
        message: message.into(),
        runtime: 0,
        endpoint: 0,
        session: 0,
        transport: 0,
        error_code: ErrorCode::Ok,
        correlation_id: 0,
    }
}

#[test]
fn slow_sink_drops_normal_logs_without_blocking_producers() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let sink_gate = Arc::clone(&gate);
    let mut logger = BoundedLogger::new(
        LoggerConfig {
            normal_capacity: 1,
            error_capacity: 1,
        },
        move |entry| {
            if entry.message == "block" {
                let (lock, wake) = &*sink_gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = wake.wait(open).unwrap();
                }
            }
        },
    )
    .unwrap();

    assert!(logger.log(record(LogLevel::Info, "block")));
    std::thread::sleep(Duration::from_millis(20));
    for index in 0..20 {
        logger.log(record(LogLevel::Info, &format!("normal-{index}")));
    }
    assert!(logger.dropped() > 0);

    let (lock, wake) = &*gate;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    logger.shutdown();
}

#[test]
fn error_log_has_reserved_capacity_when_normal_queue_is_full() {
    let errors_seen = Arc::new(AtomicUsize::new(0));
    let sink_errors = Arc::clone(&errors_seen);
    let mut logger = BoundedLogger::new(LoggerConfig::default(), move |entry| {
        if entry.level == LogLevel::Error {
            sink_errors.fetch_add(1, Ordering::Relaxed);
        }
    })
    .unwrap();

    for _ in 0..100 {
        logger.log(record(LogLevel::Debug, "noise"));
    }
    assert!(logger.log(record(LogLevel::Error, "important")));
    logger.shutdown();
    assert_eq!(errors_seen.load(Ordering::Relaxed), 1);
}

#[test]
fn sink_panic_is_contained_inside_dispatch_thread() {
    let delivered_after_panic = Arc::new(AtomicBool::new(false));
    let delivered = Arc::clone(&delivered_after_panic);
    let mut logger = BoundedLogger::new(LoggerConfig::default(), move |entry| {
        if entry.message == "panic" {
            panic!("sink failure");
        }
        if entry.message == "after" {
            delivered.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();

    assert!(logger.log(record(LogLevel::Warn, "panic")));
    assert!(logger.log(record(LogLevel::Warn, "after")));
    logger.shutdown();
    assert_eq!(logger.sink_panics(), 1);
    assert!(delivered_after_panic.load(Ordering::Relaxed));
}

#[test]
fn zero_capacity_is_rejected() {
    let error = BoundedLogger::new(
        LoggerConfig {
            normal_capacity: 0,
            error_capacity: 1,
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn callback_latency_is_measured_without_blocking_producers() {
    let mut logger = BoundedLogger::new(LoggerConfig::default(), |_| {
        std::thread::sleep(Duration::from_millis(2));
    })
    .unwrap();

    assert!(logger.log(record(LogLevel::Info, "measure")));
    logger.shutdown();

    let latency = logger.callback_latency();
    assert_eq!(latency.sample_count, 1);
    assert!(latency.max_us >= 1_000);
}
