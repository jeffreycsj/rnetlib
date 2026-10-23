use super::{
    config::Config,
    credentials::{self, Identity},
    queue::EchoQueue,
    report::Stats,
    wire::{Disposition, Kind, Packet, Window},
};
use bytes::Bytes;
use rnet_core::{ErrorCode, RnetError, Transport};
use std::time::Duration;

fn identity() -> Identity {
    Identity {
        run: 0x0102_0304_0506_0708,
        generator: 9,
        stream: 10,
    }
}

#[test]
fn payload_has_an_independently_checked_fixed_network_order_header() {
    let bytes = Packet {
        kind: Kind::Data,
        identity: identity(),
        sequence: 0x1112_1314_1516_1718,
    }
    .encode(36);
    assert_eq!(&bytes[..12], &[82, 78, 80, 68, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(
        &bytes[20..32],
        &[0, 0, 0, 10, 17, 18, 19, 20, 21, 22, 23, 24]
    );
    assert_eq!(&bytes[32..], &[0x5a; 4]);
    assert_eq!(Packet::decode(&bytes, 36).unwrap().identity, identity());
    for index in [0, 32] {
        let mut corrupt = bytes.clone();
        corrupt[index] ^= 1;
        assert!(Packet::decode(&corrupt, 36).is_err());
    }
    assert!(Packet::decode(&bytes[..31], 36).is_err());
    assert!(Packet::decode(&bytes, 35).is_err());
}

#[test]
fn ticket_binds_run_generator_and_stream_and_rejects_tampering() {
    let token = [7; 32];
    let ticket = credentials::ticket(&token, identity());
    assert_eq!(
        credentials::authorize(&token, &ticket, identity().run).unwrap(),
        identity()
    );
    assert!(credentials::authorize(&[8; 32], &ticket, identity().run).is_err());
    assert!(credentials::authorize(&token, &ticket, identity().run + 1).is_err());
    for index in [0, 8, 16, 30] {
        let mut corrupt = ticket.clone();
        corrupt[index] ^= 1;
        assert!(credentials::authorize(&token, &corrupt, identity().run).is_err());
    }
    assert!(credentials::authorize(&token, &ticket[..51], identity().run).is_err());
}

#[test]
fn udp_window_distinguishes_reordering_duplicates_and_unprovable_old_packets() {
    let mut window = Window::default();
    assert_eq!(window.observe(1, Transport::Udp).unwrap(), Disposition::New);
    assert_eq!(
        window.observe(0, Transport::Udp).unwrap(),
        Disposition::Reordered
    );
    assert_eq!(
        window.observe(0, Transport::Udp).unwrap(),
        Disposition::Duplicate
    );
    assert_eq!(
        window.observe(80, Transport::Udp).unwrap(),
        Disposition::New
    );
    assert_eq!(
        window.observe(0, Transport::Udp).unwrap(),
        Disposition::TooOld
    );
    assert!(window.observe(u64::MAX, Transport::Udp).is_err());
    let mut ordered = Window::default();
    assert!(ordered.observe(1, Transport::Tcp).is_err());
    ordered.observe(0, Transport::Tcp).unwrap();
    assert!(ordered.observe(0, Transport::Tcp).is_err());
}

#[test]
fn retry_queue_keeps_fifo_while_another_session_is_blocked() {
    let mut queue = EchoQueue::new(3, 3);
    queue.push(1, Bytes::from_static(b"a"), Kind::Data).unwrap();
    queue.push(1, Bytes::from_static(b"b"), Kind::Data).unwrap();
    queue.push(2, Bytes::from_static(b"c"), Kind::Data).unwrap();
    assert!(queue.push(3, Bytes::from_static(b"d"), Kind::Data).is_err());
    let mut accepted = Vec::new();
    queue.flush(|session, bytes| {
        if session == 1 {
            return Err(RnetError::new(ErrorCode::WouldBlock, "test pressure"));
        }
        accepted.push(bytes[0]);
        Ok(())
    });
    assert_eq!(accepted, b"c");
    assert_eq!((queue.messages, queue.bytes), (2, 2));
    queue.flush(|_, bytes| {
        accepted.push(bytes[0]);
        Ok(())
    });
    assert_eq!(accepted, b"cab");
    assert_eq!((queue.messages, queue.bytes), (0, 0));
}

#[test]
fn retry_budget_rejection_and_close_return_all_byte_reservations() {
    let mut queue = EchoQueue::new(5, 3);
    queue
        .push(1, Bytes::from_static(b"abc"), Kind::Data)
        .unwrap();
    assert!(queue.push(1, Bytes::from_static(b"d"), Kind::Data).is_err());
    assert_eq!((queue.messages, queue.bytes), (1, 3));
    assert!(queue.remove(1));
    assert_eq!((queue.messages, queue.bytes), (0, 0));
    queue.push(2, Bytes::from_static(b"a"), Kind::Ack).unwrap();
    assert!(!queue.remove(2));
}

#[test]
fn fatal_send_retains_partial_admission_and_unsent_budget() {
    let mut queue = EchoQueue::new(3, 3);
    queue.push(1, Bytes::from_static(b"a"), Kind::Data).unwrap();
    queue.push(1, Bytes::from_static(b"b"), Kind::Data).unwrap();
    let result = queue.flush(|_, bytes| {
        if bytes == b"b" {
            Err(RnetError::new(ErrorCode::InvalidArgument, "failed send"))
        } else {
            Ok(())
        }
    });
    assert!(result.error.is_some());
    assert_eq!(result.data, 1);
    assert_eq!((queue.messages, queue.bytes), (1, 1));
}

#[test]
fn keygen_must_not_accept_network_or_load_flags() {
    for flag in ["--connections", "--duration-ms", "--rate"] {
        assert!(Config::parse(
            ["keygen", "--dir", "/unused", flag, "2"]
                .into_iter()
                .map(str::to_owned)
        )
        .is_err());
    }
}

#[test]
fn configured_slo_requires_samples_and_uses_unobserved_echoes_not_network_loss() {
    let mut config = Config::parse(
        [
            "client",
            "--host",
            "localhost",
            "--port",
            "7",
            "--pin-file",
            "/unused",
            "--token-file",
            "/unused",
            "--generator-id",
            "1",
            "--run-id",
            "1",
            "--transport",
            "tcp",
            "--max-rtt-p99-us",
            "1000",
            "--max-unobserved-ppm",
            "100000",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .unwrap();
    let mut stats = Stats::default();
    assert!(stats.check_slo(&config).is_err());
    stats.rtt.record(Duration::from_micros(500));
    stats.admitted = 10;
    stats.received = 9;
    assert!(stats.check_slo(&config).is_ok());
    config.max_unobserved_ppm = Some(99999);
    assert!(stats.check_slo(&config).is_err());
}
