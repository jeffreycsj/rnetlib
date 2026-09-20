use crate::clock_sync::{
    calculate_sample, decode_clock, encode_clock, ClockPacket, ClockSyncTracker,
};
use crate::envelope::{decode, DecodedEnvelope};
use rnet_core::ErrorCode;
use std::time::{Duration, Instant};

#[test]
fn clock_control_has_fixed_protected_payload_shapes() {
    for packet in [
        ClockPacket::Probe {
            nonce: 7,
            t1_us: 101,
        },
        ClockPacket::Reply {
            nonce: 7,
            t1_us: 101,
            t2_us: 141,
            t3_us: 151,
        },
    ] {
        let wire = encode_clock(packet, 1024).expect("encode");
        let DecodedEnvelope::Control { payload, .. } = decode(&wire, 1024).expect("envelope")
        else {
            panic!("clock must stay an internal control");
        };
        assert_eq!(decode_clock(&payload).expect("decode"), packet);
    }
    for invalid in [
        Vec::new(),
        vec![1; 16],
        vec![1; 18],
        vec![2; 32],
        vec![2; 34],
        vec![3; 17],
    ] {
        assert_eq!(
            decode_clock(&invalid)
                .expect_err("invalid clock control")
                .code(),
            ErrorCode::ProtocolError
        );
    }
}

#[test]
fn four_timestamps_yield_signed_offset_and_nonnegative_network_delay() {
    let positive = calculate_sample(100, 140, 150, 130).expect("valid sample");
    assert_eq!(positive.server_minus_client_us, 30);
    assert_eq!(positive.rtt.as_micros(), 20);
    let negative = calculate_sample(100, 80, 90, 130).expect("negative offset");
    assert_eq!(negative.server_minus_client_us, -30);
    assert_eq!(negative.rtt.as_micros(), 20);
    assert!(calculate_sample(100, 140, 139, 130).is_none());
    assert!(calculate_sample(100, 140, 150, 99).is_none());
    assert!(calculate_sample(100, 140, 190, 130).is_none());
    assert!(calculate_sample(0, u64::MAX, u64::MAX, 1).is_none());
}

#[test]
fn clock_tracker_rejects_replay_and_recovers_after_missing_reply() {
    let start = Instant::now();
    let interval = Duration::from_millis(20);
    let timeout = Duration::from_millis(100);
    let mut tracker = ClockSyncTracker::new_client();
    assert!(tracker.should_probe(start, interval, timeout));
    tracker.mark_sent(7, 100, start);
    assert!(!tracker.should_probe(start + interval, interval, timeout));
    assert!(tracker
        .accept_reply(8, 100, 140, 150, 130, start + interval, timeout)
        .is_none());
    let sample = tracker
        .accept_reply(7, 100, 140, 150, 130, start + interval, timeout)
        .expect("matched response");
    assert_eq!(sample.server_minus_client_us, 30);
    assert_eq!(sample.samples, 1);
    assert!(tracker
        .accept_reply(7, 100, 140, 150, 130, start + interval, timeout)
        .is_none());
    assert!(!tracker.should_probe(start + interval, interval, timeout));
    assert!(tracker.should_probe(start + interval * 2, interval, timeout));
    tracker.mark_sent(9, 200, start + interval * 2);
    assert!(tracker.should_probe(start + interval * 2 + timeout, interval, timeout));
    assert_eq!(tracker.sample().expect("last sample").samples, 1);
}
