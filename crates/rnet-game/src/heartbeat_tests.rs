use super::envelope::{decode, ControlKind, DecodedEnvelope};
use super::heartbeat::{
    decode_heartbeat, encode_heartbeat, HeartbeatKind, HeartbeatPacket, HeartbeatTracker,
};
use rnet_core::ErrorCode;
use std::time::{Duration, Instant};

#[test]
fn heartbeat_is_an_internal_bounded_control_message() {
    for kind in [HeartbeatKind::Probe, HeartbeatKind::Ack] {
        let packet = HeartbeatPacket {
            kind,
            challenge: u64::MAX,
        };
        let encoded = encode_heartbeat(packet, 64).expect("encode heartbeat");
        let DecodedEnvelope::Control { kind, payload } =
            decode(&encoded, 64).expect("decode game envelope")
        else {
            panic!("heartbeat leaked as business data");
        };
        assert_eq!(kind, ControlKind::Heartbeat);
        assert_eq!(
            decode_heartbeat(&payload).expect("decode heartbeat"),
            packet
        );
    }
    for invalid in [&[][..], &[1][..], &[3, 0, 0, 0, 0, 0, 0, 0, 1][..]] {
        assert_eq!(
            decode_heartbeat(invalid)
                .expect_err("invalid heartbeat")
                .code(),
            ErrorCode::ProtocolError
        );
    }
    assert_eq!(
        encode_heartbeat(
            HeartbeatPacket {
                kind: HeartbeatKind::Probe,
                challenge: 1,
            },
            20,
        )
        .expect_err("bounded control")
        .code(),
        ErrorCode::MessageTooLarge
    );
}

#[test]
fn matching_ack_updates_rtt_and_jitter_from_local_monotonic_time() {
    let start = Instant::now();
    let mut tracker = HeartbeatTracker::new(Duration::from_secs(1), Duration::from_secs(3), start)
        .expect("valid policy");
    assert!(!tracker.should_probe(start + Duration::from_millis(999)));
    let first_sent = start + Duration::from_secs(1);
    assert!(tracker.should_probe(first_sent));
    tracker.mark_sent(11, first_sent).expect("first probe");
    let first = tracker
        .accept_ack(
            11,
            first_sent + Duration::from_millis(100),
            first_sent + Duration::from_millis(100),
        )
        .expect("matching ack");
    assert_eq!(first.last_rtt, Duration::from_millis(100));
    assert_eq!(first.smoothed_rtt, Duration::from_millis(100));
    assert_eq!(first.jitter, Duration::ZERO);
    assert_eq!(first.samples, 1);

    let second_sent = first_sent + Duration::from_millis(1100);
    tracker.mark_sent(12, second_sent).expect("second probe");
    let second = tracker
        .accept_ack(
            12,
            second_sent + Duration::from_millis(200),
            second_sent + Duration::from_millis(200),
        )
        .expect("second matching ack");
    assert_eq!(second.last_rtt, Duration::from_millis(200));
    assert_eq!(second.smoothed_rtt, Duration::from_micros(112_500));
    assert_eq!(second.jitter, Duration::from_micros(6_250));
    assert_eq!(second.samples, 2);
}

#[test]
fn replay_wrong_challenge_and_late_ack_cannot_refresh_liveness() {
    let start = Instant::now();
    let mut tracker = HeartbeatTracker::new(Duration::from_secs(1), Duration::from_secs(2), start)
        .expect("valid policy");
    let sent = start + Duration::from_secs(1);
    tracker.mark_sent(20, sent).expect("probe");
    assert_eq!(
        tracker.accept_ack(
            21,
            sent + Duration::from_millis(10),
            sent + Duration::from_millis(10)
        ),
        None
    );
    assert!(!tracker.timed_out(sent + Duration::from_millis(1999)));
    assert!(tracker.timed_out(sent + Duration::from_secs(2)));
    assert_eq!(
        tracker.accept_ack(
            20,
            sent + Duration::from_secs(2),
            sent + Duration::from_secs(2)
        ),
        None
    );
    assert!(tracker.timed_out(sent + Duration::from_secs(3)));
}

#[test]
fn persistent_send_backpressure_cannot_keep_a_session_alive_without_ack() {
    let start = Instant::now();
    let tracker = HeartbeatTracker::new(Duration::from_secs(1), Duration::from_secs(2), start)
        .expect("valid policy");
    assert!(tracker.should_probe(start + Duration::from_secs(1)));
    assert!(!tracker.timed_out(start + Duration::from_millis(2999)));
    assert!(tracker.timed_out(start + Duration::from_secs(3)));
}

#[test]
fn probe_policy_rejects_invalid_intervals_and_concurrent_probes() {
    let start = Instant::now();
    for (interval, timeout) in [
        (Duration::ZERO, Duration::from_secs(1)),
        (Duration::from_secs(1), Duration::ZERO),
        (Duration::from_secs(2), Duration::from_secs(1)),
    ] {
        assert_eq!(
            HeartbeatTracker::new(interval, timeout, start)
                .expect_err("invalid policy")
                .code(),
            ErrorCode::InvalidArgument
        );
    }
    let mut tracker = HeartbeatTracker::new(Duration::from_secs(1), Duration::from_secs(2), start)
        .expect("valid policy");
    assert_eq!(
        tracker
            .mark_sent(1, start)
            .expect_err("probe too early")
            .code(),
        ErrorCode::InvalidState
    );
    tracker
        .mark_sent(1, start + Duration::from_secs(1))
        .expect("due probe");
    assert_eq!(
        tracker
            .mark_sent(2, start + Duration::from_secs(2))
            .expect_err("probe already pending")
            .code(),
        ErrorCode::InvalidState
    );
}

#[test]
fn repeated_inbound_probes_are_rate_limited_per_session() {
    let start = Instant::now();
    let mut tracker = HeartbeatTracker::new(Duration::from_secs(4), Duration::from_secs(12), start)
        .expect("valid policy");
    assert!(tracker.admit_probe(start));
    assert!(!tracker.admit_probe(start + Duration::from_millis(999)));
    assert!(tracker.admit_probe(start + Duration::from_secs(1)));
}
