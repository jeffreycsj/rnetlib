use bytes::BytesMut;
use rnet_transport::{KcpEngine, RustKcpEngine};

#[test]
fn kcp_recovers_a_message_after_loss_and_reordering() {
    let mut sender = RustKcpEngine::new(77).expect("sender");
    let mut receiver = RustKcpEngine::new(77).expect("receiver");
    let payload = vec![42_u8; 4096];
    sender.send(&payload).expect("queue payload");

    let mut dropped_one = false;
    let mut received = BytesMut::new();
    for now in (0..10_000).step_by(10) {
        let mut sender_packets = Vec::new();
        sender.update(now, &mut |packet| sender_packets.push(packet.to_vec()));
        sender_packets.reverse();
        for packet in sender_packets {
            if !dropped_one {
                dropped_one = true;
                continue;
            }
            receiver.input(&packet, now).expect("receiver input");
        }

        let mut receiver_packets = Vec::new();
        receiver.update(now, &mut |packet| receiver_packets.push(packet.to_vec()));
        for packet in receiver_packets {
            sender.input(&packet, now).expect("sender input");
        }
        if receiver.recv(&mut received).expect("receive") == Some(payload.len()) {
            assert_eq!(received.as_ref(), payload.as_slice());
            assert!(sender.observed_rtt().is_some());
            return;
        }
    }
    panic!("KCP did not recover the payload");
}

#[test]
fn kcp_packets_honor_the_runtime_mtu() {
    let mut sender = RustKcpEngine::new_with_mtu(88, 576).expect("sender");
    sender.send(&vec![7_u8; 4096]).expect("queue payload");

    let mut packets = Vec::new();
    sender.update(0, &mut |packet| packets.push(packet.to_vec()));

    assert!(!packets.is_empty());
    assert!(packets.iter().all(|packet| packet.len() <= 576));
}

#[test]
fn kcp_rejects_payloads_beyond_the_unacknowledged_byte_budget() {
    let mut sender = RustKcpEngine::new_with_limits(99, 576, 1000).expect("sender");
    sender.send(&vec![1_u8; 400]).expect("first payload");

    let error = sender
        .send(&vec![2_u8; 400])
        .expect_err("second payload must exceed the KCP budget");

    assert_eq!(error.code(), rnet_core::ErrorCode::WouldBlock);
}

#[test]
fn malformed_kcp_headers_cannot_panic_the_process() {
    // These packets are minimized regression inputs from the KCP fuzz target. They exercise ACK
    // time arithmetic, fragment arithmetic, sequence comparison, and peer-window validation.
    let packets: &[&[u8]] = &[
        &[
            0x01, 0x00, 0x00, 0x00, 0x52, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x80, 0x1c, 0x00,
            0x1c, 0x00, 0x03, 0xf3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x01, 0x00, 0x00,
            0x00,
        ],
        &[
            0x01, 0x00, 0x00, 0x00, 0x51, 0x00, 0x02, 0xfc, 0x00, 0x1a, 0x32, 0xff, 0x00, 0x00,
            0x00, 0x80, 0xfa, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
        &[
            0x01, 0x00, 0x00, 0x00, 0x52, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x09, 0x00,
            0xff, 0xff, 0x00, 0x00, 0xfa, 0x04, 0x00, 0x00, 0x00, 0x00,
        ],
    ];

    for packet in packets {
        let mut engine = RustKcpEngine::new_with_mtu(1, 586).expect("engine");
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.input(packet, 255)));

        assert!(result.is_ok(), "malformed network input must not unwind");
        assert_eq!(
            result.unwrap().unwrap_err().code(),
            rnet_core::ErrorCode::ProtocolError
        );
    }
}

#[test]
fn kcp_accepts_valid_serials_across_the_signed_boundary() {
    let mut packet = Vec::with_capacity(24);
    packet.extend_from_slice(&1_u32.to_le_bytes());
    packet.push(82); // ACK
    packet.push(0);
    packet.extend_from_slice(&128_u16.to_le_bytes());
    packet.extend_from_slice(&0x7fff_fff0_u32.to_le_bytes());
    packet.extend_from_slice(&0x8000_0000_u32.to_le_bytes());
    packet.extend_from_slice(&0x8000_0000_u32.to_le_bytes());
    packet.extend_from_slice(&0_u32.to_le_bytes());
    let mut engine = RustKcpEngine::new(1).expect("engine");

    engine
        .input(&packet, 0x8000_0010)
        .expect("the full KCP serial number space must remain usable");
}
