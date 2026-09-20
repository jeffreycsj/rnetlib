//! Game-specific network quality sampling. UDP loss is inferred from library-owned data sequence.

use crate::event::{QualityBasis, QualityGrade};
use crate::heartbeat::QualitySample;
use rnet_transport::KcpRetransmissionSnapshot;
use std::time::Duration;

const REORDER_WINDOW: u32 = 64;
const MAX_FORWARD_GAP: u32 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UdpObservation {
    First,
    New,
    Reordered,
    Duplicate,
    TooOld,
    Discontinuity,
}

/// Receiver-side sequence gap estimate, not a raw IP-layer packet capture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UdpLossSnapshot {
    pub expected: u64,
    pub received: u64,
    pub missing: u64,
    pub recent_expected: u8,
    pub recent_missing: u8,
    pub recent_loss_per_mille: u16,
    pub reordered: u64,
    pub duplicates: u64,
    pub too_old: u64,
    pub discontinuities: u64,
}

#[derive(Debug, Default)]
pub(crate) struct UdpSessionQuality {
    pub next_outbound_sequence: u32,
    pub inbound: UdpLossTracker,
}

/// A bounded reorder window tolerates late datagrams without retaining per-packet state.
#[derive(Debug, Default)]
pub(crate) struct UdpLossTracker {
    highest: Option<u32>,
    seen: u64,
    span: u64,
    expected: u64,
    received: u64,
    reordered: u64,
    duplicates: u64,
    too_old: u64,
    discontinuities: u64,
}

impl UdpLossTracker {
    pub(crate) fn observe(&mut self, sequence: u32) -> UdpObservation {
        let Some(highest) = self.highest else {
            self.highest = Some(sequence);
            self.seen = 1;
            self.span = 1;
            self.expected = 1;
            self.received = 1;
            return UdpObservation::First;
        };
        let forward = sequence.wrapping_sub(highest);
        if forward == 0 {
            self.duplicates = self.duplicates.saturating_add(1);
            return UdpObservation::Duplicate;
        }
        if forward < (1 << 31) {
            if forward > MAX_FORWARD_GAP {
                // A corrupt or malicious jump must not make the quality estimate permanently
                // report nearly 100% loss. Restart only the recent window, retaining totals.
                self.highest = Some(sequence);
                self.seen = 1;
                self.span = 1;
                self.expected = self.expected.saturating_add(1);
                self.received = self.received.saturating_add(1);
                self.discontinuities = self.discontinuities.saturating_add(1);
                return UdpObservation::Discontinuity;
            }
            self.highest = Some(sequence);
            self.seen = if forward >= REORDER_WINDOW {
                1
            } else {
                (self.seen << forward) | 1
            };
            self.span = self.span.saturating_add(u64::from(forward));
            self.expected = self.expected.saturating_add(u64::from(forward));
            self.received = self.received.saturating_add(1);
            return UdpObservation::New;
        }
        let behind = highest.wrapping_sub(sequence);
        if behind >= REORDER_WINDOW || u64::from(behind) >= self.span {
            self.too_old = self.too_old.saturating_add(1);
            return UdpObservation::TooOld;
        }
        let bit = 1_u64 << behind;
        if self.seen & bit != 0 {
            self.duplicates = self.duplicates.saturating_add(1);
            return UdpObservation::Duplicate;
        }
        self.seen |= bit;
        self.received = self.received.saturating_add(1);
        self.reordered = self.reordered.saturating_add(1);
        UdpObservation::Reordered
    }

    pub(crate) fn snapshot(&self) -> UdpLossSnapshot {
        let recent_expected = self.span.min(u64::from(REORDER_WINDOW));
        let recent_received = u64::from(self.seen.count_ones()).min(recent_expected);
        let recent_missing = recent_expected.saturating_sub(recent_received);
        UdpLossSnapshot {
            expected: self.expected,
            received: self.received,
            missing: self.expected.saturating_sub(self.received),
            recent_expected: recent_expected as u8,
            recent_missing: recent_missing as u8,
            recent_loss_per_mille: recent_missing
                .saturating_mul(1000)
                .checked_div(recent_expected)
                .unwrap_or(0) as u16,
            reordered: self.reordered,
            duplicates: self.duplicates,
            too_old: self.too_old,
            discontinuities: self.discontinuities,
        }
    }
}

/// Tunable thresholds for a game-facing grade. RTT and jitter use authenticated heartbeat
/// samples. UDP data gaps are advisory when the server selects plaintext business records:
/// unlike heartbeat controls, those records then have no integrity protection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QualityPolicy {
    pub minimum_rtt_samples: u64,
    pub minimum_udp_packets: u8,
    pub minimum_kcp_segments: u16,
    pub excellent_rtt: Duration,
    pub good_rtt: Duration,
    pub fair_rtt: Duration,
    pub excellent_jitter: Duration,
    pub good_jitter: Duration,
    pub fair_jitter: Duration,
    pub excellent_udp_loss_per_mille: u16,
    pub good_udp_loss_per_mille: u16,
    pub fair_udp_loss_per_mille: u16,
    pub excellent_kcp_retransmission_per_mille: u16,
    pub good_kcp_retransmission_per_mille: u16,
    pub fair_kcp_retransmission_per_mille: u16,
}

impl Default for QualityPolicy {
    fn default() -> Self {
        Self {
            minimum_rtt_samples: 3,
            minimum_udp_packets: 16,
            minimum_kcp_segments: 16,
            excellent_rtt: Duration::from_millis(80),
            good_rtt: Duration::from_millis(150),
            fair_rtt: Duration::from_millis(250),
            excellent_jitter: Duration::from_millis(15),
            good_jitter: Duration::from_millis(35),
            fair_jitter: Duration::from_millis(70),
            excellent_udp_loss_per_mille: 10,
            good_udp_loss_per_mille: 30,
            fair_udp_loss_per_mille: 100,
            excellent_kcp_retransmission_per_mille: 10,
            good_kcp_retransmission_per_mille: 50,
            fair_kcp_retransmission_per_mille: 150,
        }
    }
}

impl QualityPolicy {
    pub(crate) fn is_valid(self) -> bool {
        self.minimum_rtt_samples > 0
            && (1..=64).contains(&self.minimum_udp_packets)
            && (1..=128).contains(&self.minimum_kcp_segments)
            && self.excellent_rtt <= self.good_rtt
            && self.good_rtt <= self.fair_rtt
            && self.excellent_jitter <= self.good_jitter
            && self.good_jitter <= self.fair_jitter
            && self.excellent_udp_loss_per_mille <= self.good_udp_loss_per_mille
            && self.good_udp_loss_per_mille <= self.fair_udp_loss_per_mille
            && self.fair_udp_loss_per_mille <= 1000
            && self.excellent_kcp_retransmission_per_mille <= self.good_kcp_retransmission_per_mille
            && self.good_kcp_retransmission_per_mille <= self.fair_kcp_retransmission_per_mille
            && self.fair_kcp_retransmission_per_mille <= 1000
    }
}

pub(crate) fn classify(
    sample: QualitySample,
    udp_loss: Option<UdpLossSnapshot>,
    kcp: Option<KcpRetransmissionSnapshot>,
    policy: QualityPolicy,
) -> (QualityGrade, QualityBasis) {
    let usable_loss = udp_loss.filter(|loss| loss.recent_expected >= policy.minimum_udp_packets);
    let usable_kcp = kcp.filter(|stats| stats.recent_segments >= policy.minimum_kcp_segments);
    let basis = if usable_loss.is_some() {
        QualityBasis::UdpSequenceGap
    } else if usable_kcp.is_some() {
        QualityBasis::KcpRetransmission
    } else {
        QualityBasis::LatencyOnly
    };
    if sample.samples < policy.minimum_rtt_samples {
        return (QualityGrade::Unknown, basis);
    }
    let mut grade = grade_for(
        sample.smoothed_rtt,
        policy.excellent_rtt,
        policy.good_rtt,
        policy.fair_rtt,
    )
    .max(grade_for(
        sample.jitter,
        policy.excellent_jitter,
        policy.good_jitter,
        policy.fair_jitter,
    ));
    if let Some(loss) = usable_loss {
        grade = grade.max(grade_for(
            loss.recent_loss_per_mille,
            policy.excellent_udp_loss_per_mille,
            policy.good_udp_loss_per_mille,
            policy.fair_udp_loss_per_mille,
        ));
    }
    if let Some(stats) = usable_kcp {
        grade = grade.max(grade_for(
            stats.recent_retransmission_per_mille,
            policy.excellent_kcp_retransmission_per_mille,
            policy.good_kcp_retransmission_per_mille,
            policy.fair_kcp_retransmission_per_mille,
        ));
    }
    (grade, basis)
}

fn grade_for<T: Ord>(value: T, excellent: T, good: T, fair: T) -> QualityGrade {
    if value <= excellent {
        QualityGrade::Excellent
    } else if value <= good {
        QualityGrade::Good
    } else if value <= fair {
        QualityGrade::Fair
    } else {
        QualityGrade::Poor
    }
}

/// Suppresses one-sample quality spikes and repeated notifications at a stable grade.
#[derive(Debug, Default)]
pub(crate) struct QualityChangeGate {
    published: Option<(QualityGrade, QualityBasis)>,
    candidate: Option<(QualityGrade, QualityBasis)>,
}

impl QualityChangeGate {
    pub(crate) fn observe(&mut self, grade: QualityGrade, basis: QualityBasis) -> bool {
        if grade == QualityGrade::Unknown || self.published == Some((grade, basis)) {
            self.candidate = None;
            return false;
        }
        let next = (grade, basis);
        if self.candidate == Some(next) {
            self.published = Some(next);
            self.candidate = None;
            true
        } else {
            self.candidate = Some(next);
            false
        }
    }
}
