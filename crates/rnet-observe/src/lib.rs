//! Bounded asynchronous logging and metrics helpers.
//!
//! Forward owned [`LogRecord`] values into an application logger through [`BoundedLogger`].
//! Logging is best-effort and must not be used as a reliable audit/event delivery channel.

#![deny(missing_docs)]

use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Handle, Result, RnetError};

const LATENCY_BUCKET_COUNT: usize = 66;

/// Fixed-memory latency summary in microseconds. Percentiles are power-of-two bucket estimates
/// capped by the observed maximum, not exact order statistics. The producer defines whether the
/// snapshot is cumulative or windowed; `sample_count == 0` means no samples, not zero latency.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LatencySnapshot {
    /// Number of observations represented by the sampled buckets, saturating at `u64::MAX`.
    pub sample_count: u64,
    /// Estimated median latency in microseconds.
    pub p50_us: u64,
    /// Estimated 90th percentile latency in microseconds.
    pub p90_us: u64,
    /// Estimated 95th percentile latency in microseconds.
    pub p95_us: u64,
    /// Estimated 99th percentile latency in microseconds.
    pub p99_us: u64,
    /// Estimated 99.9th percentile latency in microseconds.
    pub p999_us: u64,
    /// Largest recorded duration in microseconds, not a percentile bucket boundary.
    pub max_us: u64,
}

/// Concurrent, fixed-size histogram with no per-sample allocation.
/// Bucket/max reads are independent; snapshots and resets are not whole-histogram transactions.
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
    /// `duration` is truncated to whole microseconds and capped at `u64::MAX` microseconds;
    /// zero is a valid observation. The caller chooses the operation being measured.
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

    /// Reads observations since construction or the last reset, without consuming them.
    /// Concurrent record/reset calls may be reflected in different buckets at different instants.
    pub fn snapshot(&self) -> LatencySnapshot {
        let counts: [u64; LATENCY_BUCKET_COUNT] =
            std::array::from_fn(|index| self.buckets[index].load(Ordering::Relaxed));
        snapshot_from_counts(counts, self.max_us.load(Ordering::Relaxed))
    }

    /// Returns the interval accumulated since the previous drain and begins a new interval.
    /// Coordinate consumers: each call consumes shared counters. Individual swaps are atomic,
    /// but a racing observation's bucket and maximum may fall into different interval snapshots.
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

/// Structured log severity. Only `Error` uses the logger's separate reserved queue.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    /// Fine-grained diagnostic tracing.
    Trace = 0,
    /// Developer-oriented diagnostic detail.
    Debug = 1,
    /// Expected lifecycle or operational information.
    Info = 2,
    /// Recoverable failure or degraded behavior; shares normal queue capacity.
    Warn = 3,
    /// Failure diagnostics using separately reserved queue capacity.
    Error = 4,
}

/// Owned structured record delivered to a logging sink. RNet-generated messages omit credentials
/// and business payloads; callers constructing their own records are responsible for redaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogRecord {
    /// Producer's wall-clock milliseconds since Unix epoch; not a monotonic duration.
    pub timestamp_unix_ms: u64,
    /// Severity used for queue selection, not automatic minimum-level filtering.
    pub level: LogLevel,
    /// Stable event category suitable for querying, independent of free-form diagnostic text.
    pub event_name: String,
    /// Component/category emitting the record, such as `rnet.game`.
    pub target: String,
    /// Human-readable diagnostic detail; do not treat its wording as a stable wire format.
    pub message: String,
    /// Producer's runtime correlation identity; not necessarily an FFI runtime handle.
    pub runtime: Handle,
    /// Local endpoint context, or zero when not applicable/available.
    pub endpoint: Handle,
    /// Local session context, or zero when not applicable/available.
    pub session: Handle,
    /// Transport tag: 1 TCP, 2 UDP, 3 KCP; zero when unknown or not applicable.
    pub transport: u32,
    /// Structured local error category; `Ok` when the record does not describe an error.
    pub error_code: ErrorCode,
    /// Optional application request/trace correlation; zero means absent.
    pub correlation_id: u64,
}

/// Logger queue capacities in records, not bytes. Both must be positive.
/// Separate error capacity limits starvation but cannot guarantee lossless logging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoggerConfig {
    /// Queue slots shared by trace/debug/info/warn records; default 256.
    pub normal_capacity: usize,
    /// Slots reserved for error records; default 16.
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

#[derive(Default)]
struct CallbackLatency {
    total: LatencyHistogram,
    window: LatencyHistogram,
}

/// Best-effort logger with one dedicated callback thread and two bounded queues.
/// Producers may log concurrently; callbacks execute serially, away from I/O/game-poll threads.
/// Cross-severity ordering is not guaranteed because errors use a separate queue.
/// Dropping the logger drains admitted records and joins the callback thread; a blocked callback
/// can therefore block destruction. Never shut down/drop the logger or owning runtime in its sink.
pub struct BoundedLogger {
    normal: Option<SyncSender<LogRecord>>,
    errors: Option<SyncSender<LogRecord>>,
    dropped: Arc<AtomicU64>,
    sink_panics: Arc<AtomicU64>,
    callback_latency: Arc<CallbackLatency>,
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
    /// Starts a logger with `config` queue capacities and a caller-supplied `sink`.
    /// The sink receives owned records and may forward them to the application's log component.
    /// It must return promptly; panics are caught and counted. Captured data must remain valid for
    /// the logger's lifetime (`'static`). Zero capacity returns `InvalidArgument`; thread creation
    /// failures return an I/O error. This constructor does not install the logger on a runtime.
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
        let callback_latency = Arc::new(CallbackLatency::default());
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
    /// Consumes `record`; `true` means queued, not persisted. `false` means it was discarded due
    /// to full/disconnected queues or shutdown and counted by [`Self::dropped`]. No level filter.
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

    /// Cumulative records rejected by queue admission, including attempts after shutdown.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Cumulative caught callback panics; logging continues with later records.
    pub fn sink_panics(&self) -> u64 {
        self.sink_panics.load(Ordering::Relaxed)
    }

    /// Cumulative callback execution time, including callbacks that panicked; no reset.
    /// Excludes time waiting in the logging queues; units and sampling follow [`LatencySnapshot`].
    pub fn callback_latency(&self) -> LatencySnapshot {
        self.callback_latency.total.snapshot()
    }

    /// Rotates callback latency samples without resetting the cumulative summary.
    /// Consumes the shared window, with the concurrency caveats of [`LatencyHistogram::snapshot_and_reset`].
    pub fn drain_callback_latency_window(&self) -> LatencySnapshot {
        self.callback_latency.window.snapshot_and_reset()
    }

    /// Stops admission, drains previously queued records and joins the sink thread.
    /// Repeated calls are harmless. There is no drain deadline: a slow sink delays this call.
    /// Never invoke from the sink itself, including indirectly through owning runtime destruction.
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
    callback_latency: Arc<CallbackLatency>,
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
    callback_latency: &CallbackLatency,
) {
    let started = Instant::now();
    if catch_unwind(AssertUnwindSafe(|| sink(record))).is_err() {
        sink_panics.fetch_add(1, Ordering::Relaxed);
    }
    let elapsed = started.elapsed();
    callback_latency.window.record(elapsed);
    callback_latency.total.record(elapsed);
}
