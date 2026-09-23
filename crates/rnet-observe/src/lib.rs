//! Bounded asynchronous logging and metrics helpers.

use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Handle, Result, RnetError};

const LATENCY_BUCKET_COUNT: usize = 66;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LatencySnapshot {
    pub sample_count: u64,
    pub p50_us: u64,
    pub p90_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
}

#[derive(Debug)]
pub struct LatencyHistogram {
    buckets: [AtomicU64; LATENCY_BUCKET_COUNT],
    max_us: AtomicU64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; LATENCY_BUCKET_COUNT],
            max_us: AtomicU64::new(0),
        }
    }
}

impl LatencyHistogram {
    /// Records a latency using fixed power-of-two microsecond buckets.
    // `try_update` is the future spelling, but it is unavailable on the declared Rust 1.85 MSRV.
    #[allow(deprecated)]
    pub fn record(&self, duration: Duration) {
        let micros = duration.as_micros().min(u128::from(u64::MAX)) as u64;
        let bucket = latency_bucket(micros);
        let _ = self.buckets[bucket].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(1))
        });
        self.max_us.fetch_max(micros, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> LatencySnapshot {
        let counts: [u64; LATENCY_BUCKET_COUNT] =
            std::array::from_fn(|index| self.buckets[index].load(Ordering::Relaxed));
        snapshot_from_counts(counts, self.max_us.load(Ordering::Relaxed))
    }

    /// Returns the interval accumulated since the previous drain and begins a new interval.
    pub fn snapshot_and_reset(&self) -> LatencySnapshot {
        let counts: [u64; LATENCY_BUCKET_COUNT] =
            std::array::from_fn(|index| self.buckets[index].swap(0, Ordering::AcqRel));
        snapshot_from_counts(counts, self.max_us.swap(0, Ordering::AcqRel))
    }
}

fn snapshot_from_counts(counts: [u64; LATENCY_BUCKET_COUNT], max_us: u64) -> LatencySnapshot {
    let sample_count = counts.iter().copied().fold(0_u64, u64::saturating_add);
    LatencySnapshot {
        sample_count,
        p50_us: percentile(&counts, sample_count, 500, 1_000).min(max_us),
        p90_us: percentile(&counts, sample_count, 900, 1_000).min(max_us),
        p95_us: percentile(&counts, sample_count, 950, 1_000).min(max_us),
        p99_us: percentile(&counts, sample_count, 990, 1_000).min(max_us),
        p999_us: percentile(&counts, sample_count, 999, 1_000).min(max_us),
        max_us,
    }
}

fn latency_bucket(micros: u64) -> usize {
    if micros == 0 {
        0
    } else if micros == u64::MAX {
        LATENCY_BUCKET_COUNT - 1
    } else {
        1 + (u64::BITS - (micros - 1).leading_zeros()) as usize
    }
}

fn bucket_upper_bound(index: usize) -> u64 {
    match index {
        0 => 0,
        1..=64 => 1_u64 << (index - 1),
        _ => u64::MAX,
    }
}

fn percentile(
    counts: &[u64; LATENCY_BUCKET_COUNT],
    total: u64,
    numerator: u64,
    denominator: u64,
) -> u64 {
    if total == 0 {
        return 0;
    }
    let target = ((u128::from(total) * u128::from(numerator))
        .saturating_add(u128::from(denominator - 1))
        / u128::from(denominator)) as u64;
    let mut seen = 0_u64;
    for (index, count) in counts.iter().copied().enumerate() {
        seen = seen.saturating_add(count);
        if seen >= target {
            return bucket_upper_bound(index);
        }
    }
    u64::MAX
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogRecord {
    pub timestamp_unix_ms: u64,
    pub level: LogLevel,
    pub event_name: String,
    pub target: String,
    pub message: String,
    pub runtime: Handle,
    pub endpoint: Handle,
    pub session: Handle,
    pub transport: u32,
    pub error_code: ErrorCode,
    pub correlation_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoggerConfig {
    pub normal_capacity: usize,
    pub error_capacity: usize,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            normal_capacity: 256,
            error_capacity: 16,
        }
    }
}

pub struct BoundedLogger {
    normal: Option<SyncSender<LogRecord>>,
    errors: Option<SyncSender<LogRecord>>,
    dropped: Arc<AtomicU64>,
    sink_panics: Arc<AtomicU64>,
    callback_latency: Arc<LatencyHistogram>,
    dispatch: Option<JoinHandle<()>>,
}

impl fmt::Debug for BoundedLogger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedLogger")
            .field("dropped", &self.dropped())
            .field("sink_panics", &self.sink_panics())
            .finish_non_exhaustive()
    }
}

impl BoundedLogger {
    pub fn new(config: LoggerConfig, sink: impl Fn(LogRecord) + Send + 'static) -> Result<Self> {
        if config.normal_capacity == 0 || config.error_capacity == 0 {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "logger capacities must be positive",
            ));
        }
        let (normal_tx, normal_rx) = mpsc::sync_channel(config.normal_capacity);
        let (error_tx, error_rx) = mpsc::sync_channel(config.error_capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let sink_panics = Arc::new(AtomicU64::new(0));
        let dispatch_panics = Arc::clone(&sink_panics);
        let callback_latency = Arc::new(LatencyHistogram::default());
        let dispatch_latency = Arc::clone(&callback_latency);
        let dispatch = thread::Builder::new()
            .name("rnet-logger".into())
            .spawn(move || {
                dispatch_logs(normal_rx, error_rx, sink, dispatch_panics, dispatch_latency)
            })
            .map_err(RnetError::from)?;
        Ok(Self {
            normal: Some(normal_tx),
            errors: Some(error_tx),
            dropped,
            sink_panics,
            callback_latency,
            dispatch: Some(dispatch),
        })
    }

    /// Enqueues a record without waiting. Error records use separately reserved capacity.
    pub fn log(&self, record: LogRecord) -> bool {
        let sender = if record.level == LogLevel::Error {
            self.errors.as_ref()
        } else {
            self.normal.as_ref()
        };
        let Some(sender) = sender else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        match sender.try_send(record) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn sink_panics(&self) -> u64 {
        self.sink_panics.load(Ordering::Relaxed)
    }

    pub fn callback_latency(&self) -> LatencySnapshot {
        self.callback_latency.snapshot()
    }

    pub fn shutdown(&mut self) {
        self.normal.take();
        self.errors.take();
        if let Some(dispatch) = self.dispatch.take() {
            let _ = dispatch.join();
        }
    }
}

impl Drop for BoundedLogger {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn dispatch_logs(
    normal: Receiver<LogRecord>,
    errors: Receiver<LogRecord>,
    sink: impl Fn(LogRecord),
    sink_panics: Arc<AtomicU64>,
    callback_latency: Arc<LatencyHistogram>,
) {
    let mut normal_open = true;
    let mut error_open = true;
    while normal_open || error_open {
        match errors.try_recv() {
            Ok(record) => deliver(&sink, record, &sink_panics, &callback_latency),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => error_open = false,
        }

        if normal_open {
            match normal.recv_timeout(Duration::from_millis(5)) {
                Ok(record) => deliver(&sink, record, &sink_panics, &callback_latency),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => normal_open = false,
            }
        } else if error_open {
            match errors.recv_timeout(Duration::from_millis(5)) {
                Ok(record) => deliver(&sink, record, &sink_panics, &callback_latency),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => error_open = false,
            }
        }
    }
}

fn deliver(
    sink: &impl Fn(LogRecord),
    record: LogRecord,
    sink_panics: &AtomicU64,
    callback_latency: &LatencyHistogram,
) {
    let started = Instant::now();
    if catch_unwind(AssertUnwindSafe(|| sink(record))).is_err() {
        sink_panics.fetch_add(1, Ordering::Relaxed);
    }
    callback_latency.record(started.elapsed());
}
