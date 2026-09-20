use super::event::{QualityBasis, QualityGrade};
use super::heartbeat::QualitySample;
use super::quality::{classify, QualityChangeGate, QualityPolicy, UdpLossTracker, UdpObservation};
use rnet_transport::KcpRetransmissionSnapshot;
use std::time::Duration;

#[test]
fn udp_loss_tracks_gaps_late_arrivals_and_duplicates() {
    let mut tracker = UdpLossTracker::default();
    assert_eq!(tracker.observe(10), UdpObservation::First);
    assert_eq!(tracker.observe(12), UdpObservation::New);
    let loss = tracker.snapshot();
    assert_eq!((loss.expected, loss.received, loss.missing), (3, 2, 1));
    assert_eq!(loss.recent_loss_per_mille, 333);

    assert_eq!(tracker.observe(11), UdpObservation::Reordered);
    assert_eq!(tracker.observe(11), UdpObservation::Duplicate);
    let loss = tracker.snapshot();
    assert_eq!((loss.expected, loss.received, loss.missing), (3, 3, 0));
    assert_eq!(loss.recent_loss_per_mille, 0);
    assert_eq!((loss.reordered, loss.duplicates), (1, 1));
}

#[test]
fn udp_loss_handles_wraparound_and_bounds_untrusted_jumps() {
    let mut tracker = UdpLossTracker::default();
    assert_eq!(tracker.observe(u32::MAX - 1), UdpObservation::First);
    assert_eq!(tracker.observe(0), UdpObservation::New);
    assert_eq!(tracker.snapshot().missing, 1);
    assert_eq!(tracker.observe(u32::MAX), UdpObservation::Reordered);
    assert_eq!(tracker.snapshot().missing, 0);

    assert_eq!(tracker.observe(10_000), UdpObservation::Discontinuity);
    assert_eq!(tracker.observe(9_999), UdpObservation::TooOld);
    let loss = tracker.snapshot();
    assert_eq!(loss.discontinuities, 1);
    assert_eq!(loss.recent_loss_per_mille, 0);
    assert_eq!(loss.expected, 4);
    assert_eq!(loss.received, 4);
}

#[test]
fn udp_loss_ignores_packets_older_than_the_reorder_window() {
    let mut tracker = UdpLossTracker::default();
    assert_eq!(tracker.observe(1), UdpObservation::First);
    for sequence in 2..=70 {
        assert_eq!(tracker.observe(sequence), UdpObservation::New);
    }
    assert_eq!(tracker.observe(1), UdpObservation::TooOld);
    let loss = tracker.snapshot();
    assert_eq!((loss.expected, loss.received, loss.missing), (70, 70, 0));
    assert_eq!(loss.too_old, 1);
}

#[test]
fn quality_grade_requires_enough_samples_and_uses_worst_available_signal() {
    let policy = QualityPolicy::default();
    let mut sample = QualitySample {
        last_rtt: Duration::from_millis(40),
        smoothed_rtt: Duration::from_millis(40),
        jitter: Duration::from_millis(5),
        samples: 2,
    };
    assert_eq!(
        classify(sample, None, None, policy),
        (QualityGrade::Unknown, QualityBasis::LatencyOnly)
    );
    sample.samples = 3;
    assert_eq!(
        classify(sample, None, None, policy),
        (QualityGrade::Excellent, QualityBasis::LatencyOnly)
    );

    let mut loss = UdpLossTracker::default();
    loss.observe(0);
    loss.observe(5);
    assert_eq!(
        classify(sample, Some(loss.snapshot()), None, policy),
        (QualityGrade::Excellent, QualityBasis::LatencyOnly),
        "too few UDP packets cannot support a loss grade"
    );
    loss.observe(36);
    assert_eq!(
        classify(sample, Some(loss.snapshot()), None, policy),
        (QualityGrade::Poor, QualityBasis::UdpSequenceGap),
        "observed UDP gaps can degrade an otherwise fast link"
    );
}

#[test]
fn kcp_retransmission_grade_is_distinct_from_ip_packet_loss() {
    let sample = QualitySample {
        last_rtt: Duration::from_millis(40),
        smoothed_rtt: Duration::from_millis(40),
        jitter: Duration::from_millis(5),
        samples: 3,
    };
    let mut retransmissions = KcpRetransmissionSnapshot {
        recent_segments: 15,
        recent_retransmitted: 5,
        recent_retransmission_per_mille: 333,
        ..Default::default()
    };
    let policy = QualityPolicy::default();
    assert_eq!(
        classify(sample, None, Some(retransmissions), policy),
        (QualityGrade::Excellent, QualityBasis::LatencyOnly)
    );
    retransmissions.recent_segments = 16;
    assert_eq!(
        classify(sample, None, Some(retransmissions), policy),
        (QualityGrade::Poor, QualityBasis::KcpRetransmission)
    );
}

#[test]
fn quality_change_requires_two_consecutive_samples_and_suppresses_duplicates() {
    let mut gate = QualityChangeGate::default();
    let good = (QualityGrade::Good, QualityBasis::LatencyOnly);
    let fair = (QualityGrade::Fair, QualityBasis::UdpSequenceGap);
    assert!(!gate.observe(QualityGrade::Unknown, QualityBasis::LatencyOnly));
    assert!(!gate.observe(good.0, good.1));
    assert!(gate.observe(good.0, good.1));
    assert!(!gate.observe(good.0, good.1));
    assert!(!gate.observe(fair.0, fair.1));
    assert!(!gate.observe(good.0, good.1));
    assert!(!gate.observe(fair.0, fair.1));
    assert!(gate.observe(fair.0, fair.1));
}
