use rnet_core::ErrorCode;
use rnet_observe::LatencyHistogram;
use rnet_observe::LatencySnapshot;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Runtime-wide latency series. Discriminants are one-based; snapshot arrays use that order.
/// These are local instrumentation intervals, not end-to-end application delivery latency.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatencyKind {
    /// Successful TCP socket setup, or client UDP/KCP setup through server authorization.
    Connect = 1,
    /// Noise setup interval; the current TCP client sample also includes authorization wait.
    CryptoHandshake = 2,
    /// Waiting for a local application decision (server) or remote decision (client).
    AuthWait = 3,
    /// Outbound admission/latest replacement to worker pickup (TCP) or accepted wire send
    /// (UDP/KCP). KCP acceptance is not acknowledgement; security/backpressure may add delay.
    SendQueue = 4,
    /// Time from successful event enqueue to removal by a transport poll, excluding earlier
    /// retries waiting for queue space and application work after poll returns.
    EventQueue = 5,
    /// Latest ACK-derived KCP RTT sampled on engine updates; a value may be sampled repeatedly.
    KcpRtt = 6,
    /// KCP endpoint maintenance tick lateness, not network RTT or per-peer acknowledgement delay.
    KcpUpdateDelay = 7,
    /// Reserved logger-callback series; the bare transport does not record this series.
    /// Facade/logger telemetry must be queried from its own observation API.
    LoggerCallback = 8,
}

/// Number of series returned by the runtime's latency queries.
pub const LATENCY_KIND_COUNT: usize = 8;
pub const CLOSE_REASON_COUNT: usize = 19;
/// Number of indexed categories in [`MetricsSnapshot::admission_rejected_by_reason`].
pub const ADMISSION_REJECT_REASON_COUNT: usize = 8;

/// Resource/security reason for refused admission; discriminants index the reason array.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionRejectReason {
    /// Runtime endpoint count exhausted.
    EndpointLimit = 0,
    /// Listener's pending plus established session limit exhausted.
    EndpointSessionLimit = 1,
    /// Runtime pending-handshake budget exhausted.
    PendingHandshakeLimit = 2,
    /// Listener's active admissions for the source address/prefix exhausted.
    IpActiveLimit = 3,
    /// Listener's source address/prefix handshake token bucket exhausted.
    IpRateLimit = 4,
    /// Bounded source admission-tracking table exhausted.
    IpTableLimit = 5,
    /// Bounded datagram preflight peer/authorization capacity exhausted.
    KcpPeerLimit = 6,
    /// Datagram preflight/cookie authentication not satisfied.
    Unauthenticated = 7,
}

/// One cumulative or drained-window latency series, depending on the query that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatencyMetricSnapshot {
    /// Instrumentation interval represented by this series.
    pub kind: LatencyKind,
    /// Histogram estimates in microseconds; check `sample_count` before interpreting percentiles.
    pub latency: LatencySnapshot,
}

#[derive(Debug)]
pub(crate) struct Latencies {
    histograms: [LatencyHistogram; LATENCY_KIND_COUNT],
}

impl Default for Latencies {
    fn default() -> Self {
        Self {
            histograms: std::array::from_fn(|_| LatencyHistogram::default()),
        }
    }
}

impl Latencies {
    pub(crate) fn record(&self, kind: LatencyKind, duration: Duration) {
        self.histograms[kind as usize - 1].record(duration);
    }

    pub(crate) fn snapshot(&self) -> [LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
        const KINDS: [LatencyKind; LATENCY_KIND_COUNT] = [
            LatencyKind::Connect,
            LatencyKind::CryptoHandshake,
            LatencyKind::AuthWait,
            LatencyKind::SendQueue,
            LatencyKind::EventQueue,
            LatencyKind::KcpRtt,
            LatencyKind::KcpUpdateDelay,
            LatencyKind::LoggerCallback,
        ];
        std::array::from_fn(|index| LatencyMetricSnapshot {
            kind: KINDS[index],
            latency: self.histograms[index].snapshot(),
        })
    }

    pub(crate) fn drain_window(&self) -> [LatencyMetricSnapshot; LATENCY_KIND_COUNT] {
        const KINDS: [LatencyKind; LATENCY_KIND_COUNT] = [
            LatencyKind::Connect,
            LatencyKind::CryptoHandshake,
            LatencyKind::AuthWait,
            LatencyKind::SendQueue,
            LatencyKind::EventQueue,
            LatencyKind::KcpRtt,
            LatencyKind::KcpUpdateDelay,
            LatencyKind::LoggerCallback,
        ];
        std::array::from_fn(|index| LatencyMetricSnapshot {
            kind: KINDS[index],
            latency: self.histograms[index].snapshot_and_reset(),
        })
    }
}

/// Runtime-wide counters and resource gauges, read independently during concurrent I/O.
/// Counters accumulate for this runtime's lifetime, including after stop; gauges describe
/// retained work. Fields are not an atomic point-in-time view. These byte/frame counters
/// are intentionally not symmetric wire-traffic or peer-delivery counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetricsSnapshot {
    /// Decoded business frames before event admission; excludes protected game/security controls.
    pub frames_received: u64,
    /// Outbound frames/game controls locally admitted; includes later failures, excludes latest
    /// replacements of an already-counted slot. Does not count on-wire retransmissions.
    pub frames_sent: u64,
    /// Decoded business body bytes before event admission, excluding framing/security overhead.
    pub bytes_received: u64,
    /// Outbound frame/game-control bytes accepted by the write path, including business frame
    /// headers but excluding outer security overhead and retransmissions. Not peer-delivered bytes.
    pub bytes_sent: u64,
    /// Counted event-drop/eviction incidents; a priority insertion may evict multiple entries,
    /// so this is not an exact count of every message lost to saturation.
    pub events_dropped: u64,
    /// Send admission rejections from bounded queues/byte budgets, including caller retries.
    pub send_would_block: u64,
    /// Counted driver protocol-processing failures; not all errors or exclusively hostile input.
    pub protocol_errors: u64,
    /// Priority lifecycle notifications still rejected after bounded-queue eviction attempts.
    pub lifecycle_events_rejected: u64,
    /// Current outgoing byte reservations, including worker-held frames; not kernel/KCP buffers.
    pub queued_send_bytes: u64,
    /// Lifetime high-water mark of runtime outgoing byte reservations.
    pub peak_queued_send_bytes: u64,
    /// Current transport event payload-byte budget usage, excluding returned events/allocations.
    pub queued_event_bytes: u64,
    /// Counted refusals across all [`AdmissionRejectReason`] categories.
    pub admission_rejected: u64,
    /// Cumulative refusals indexed by the zero-based [`AdmissionRejectReason`] discriminant.
    pub admission_rejected_by_reason: [u64; ADMISSION_REJECT_REASON_COUNT],
    /// Registered listener and client endpoint handles at query time.
    pub current_endpoints: u64,
    /// Registered pending plus established session routes at query time.
    pub current_sessions: u64,
    /// Saturating difference of current sessions and pending handshakes; approximate under races.
    pub established_sessions: u64,
    /// Current reserved slots for cryptographic/application-authentication setup.
    pub pending_handshakes: u64,
    /// Lifetime high-water mark of pending-handshake reservations.
    pub peak_pending_handshakes: u64,
    /// Recorded route closures indexed by the absolute [`ErrorCode`] discriminant (`Ok` is zero).
    /// Includes failed setup; does not require successful delivery of the close event.
    pub session_closed_by_reason: [u64; CLOSE_REASON_COUNT],
}

impl MetricsSnapshot {
    /// Returns cumulative closures for `reason`; querying does not reset the counter.
    pub fn closed_sessions(&self, reason: ErrorCode) -> u64 {
        self.session_closed_by_reason[reason_index(reason)]
    }
}

#[derive(Default)]
pub(crate) struct Metrics {
    pub(crate) frames_received: AtomicU64,
    pub(crate) frames_sent: AtomicU64,
    pub(crate) bytes_received: AtomicU64,
    pub(crate) bytes_sent: AtomicU64,
    pub(crate) events_dropped: AtomicU64,
    pub(crate) send_would_block: AtomicU64,
    pub(crate) protocol_errors: AtomicU64,
    pub(crate) lifecycle_events_rejected: AtomicU64,
    pub(crate) admission_rejected: AtomicU64,
    admission_rejected_by_reason: [AtomicU64; ADMISSION_REJECT_REASON_COUNT],
    pending_handshakes: AtomicU64,
    peak_pending_handshakes: AtomicU64,
    session_closed_by_reason: [AtomicU64; CLOSE_REASON_COUNT],
}

impl Metrics {
    pub(crate) fn snapshot(
        &self,
        queued_send_bytes: usize,
        peak_queued_send_bytes: usize,
        queued_event_bytes: usize,
        current_endpoints: usize,
        current_sessions: usize,
    ) -> MetricsSnapshot {
        let pending_handshakes = self.pending_handshakes.load(Ordering::Relaxed);
        MetricsSnapshot {
            frames_received: self.frames_received.load(Ordering::Relaxed),
            frames_sent: self.frames_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
            send_would_block: self.send_would_block.load(Ordering::Relaxed),
            protocol_errors: self.protocol_errors.load(Ordering::Relaxed),
            lifecycle_events_rejected: self.lifecycle_events_rejected.load(Ordering::Relaxed),
            queued_send_bytes: queued_send_bytes.min(u64::MAX as usize) as u64,
            peak_queued_send_bytes: peak_queued_send_bytes.min(u64::MAX as usize) as u64,
            queued_event_bytes: queued_event_bytes.min(u64::MAX as usize) as u64,
            admission_rejected: self.admission_rejected.load(Ordering::Relaxed),
            admission_rejected_by_reason: std::array::from_fn(|index| {
                self.admission_rejected_by_reason[index].load(Ordering::Relaxed)
            }),
            current_endpoints: current_endpoints as u64,
            current_sessions: current_sessions as u64,
            established_sessions: (current_sessions as u64).saturating_sub(pending_handshakes),
            pending_handshakes,
            peak_pending_handshakes: self.peak_pending_handshakes.load(Ordering::Relaxed),
            session_closed_by_reason: std::array::from_fn(|index| {
                self.session_closed_by_reason[index].load(Ordering::Relaxed)
            }),
        }
    }

    pub(crate) fn record_session_closed(&self, reason: ErrorCode) {
        self.session_closed_by_reason[reason_index(reason)].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_admission_rejected(&self, reason: AdmissionRejectReason) {
        self.admission_rejected.fetch_add(1, Ordering::Relaxed);
        self.admission_rejected_by_reason[reason as usize].fetch_add(1, Ordering::Relaxed);
    }

    // `try_update` is the future spelling, but it is unavailable on the declared Rust 1.85 MSRV.
    #[allow(deprecated)]
    pub(crate) fn try_reserve_pending_handshake(&self, limit: usize) -> bool {
        let reserved =
            self.pending_handshakes
                .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                    (current < limit as u64).then_some(current + 1)
                });
        let Ok(previous) = reserved else {
            return false;
        };
        self.peak_pending_handshakes
            .fetch_max(previous + 1, Ordering::Relaxed);
        true
    }

    pub(crate) fn release_pending_handshake(&self) {
        let previous = self.pending_handshakes.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "pending handshake accounting underflow");
    }
}

fn reason_index(reason: ErrorCode) -> usize {
    (reason as i32).unsigned_abs() as usize
}
