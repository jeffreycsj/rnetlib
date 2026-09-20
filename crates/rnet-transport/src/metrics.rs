use rnet_core::ErrorCode;
use rnet_observe::LatencyHistogram;
use rnet_observe::LatencySnapshot;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatencyKind {
    Connect = 1,
    CryptoHandshake = 2,
    AuthWait = 3,
    SendQueue = 4,
    EventQueue = 5,
    KcpRtt = 6,
    KcpUpdateDelay = 7,
    LoggerCallback = 8,
}

pub const LATENCY_KIND_COUNT: usize = 8;
pub const CLOSE_REASON_COUNT: usize = 19;
pub const ADMISSION_REJECT_REASON_COUNT: usize = 8;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionRejectReason {
    EndpointLimit = 0,
    EndpointSessionLimit = 1,
    PendingHandshakeLimit = 2,
    IpActiveLimit = 3,
    IpRateLimit = 4,
    IpTableLimit = 5,
    KcpPeerLimit = 6,
    Unauthenticated = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatencyMetricSnapshot {
    pub kind: LatencyKind,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetricsSnapshot {
    pub frames_received: u64,
    pub frames_sent: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub events_dropped: u64,
    pub send_would_block: u64,
    pub protocol_errors: u64,
    pub lifecycle_events_rejected: u64,
    pub queued_send_bytes: u64,
    pub peak_queued_send_bytes: u64,
    pub queued_event_bytes: u64,
    pub admission_rejected: u64,
    pub admission_rejected_by_reason: [u64; ADMISSION_REJECT_REASON_COUNT],
    pub current_endpoints: u64,
    pub current_sessions: u64,
    pub established_sessions: u64,
    pub pending_handshakes: u64,
    pub peak_pending_handshakes: u64,
    pub session_closed_by_reason: [u64; CLOSE_REASON_COUNT],
}

impl MetricsSnapshot {
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
