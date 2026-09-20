use super::{DatagramWire, RustKcpEngine, KCP_CONV, RETRY_INTERVAL};
use crate::kcp::KcpEngine;
use crate::kcp::KcpTelemetryRegistry;
use rnet_core::Transport;
use rnet_protocol::control::{Record, RecordKind};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;

const TEST_MAX_PEERS: usize = 1024;

#[tokio::test]
async fn udp_control_record_retries_as_identical_bytes() {
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = receiver.local_addr().unwrap();
    let mut wire = DatagramWire::new(Transport::Udp, 1200, TEST_MAX_PEERS);
    let record = Record::new(RecordKind::ClientHello, 0, b"cookie");

    wire.send_reliable(&sender, peer, &record, 128)
        .await
        .unwrap();
    let mut first = [0; 256];
    let first_len = receiver.recv(&mut first).await.unwrap();
    tokio::time::sleep(RETRY_INTERVAL + Duration::from_millis(10)).await;
    assert!(wire.retry(&sender).await.is_empty());
    let mut second = [0; 256];
    let second_len = receiver.recv(&mut second).await.unwrap();

    assert_eq!(&first[..first_len], &second[..second_len]);
}

#[tokio::test]
async fn duplicate_control_uses_cached_ciphertext_response() {
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = receiver.local_addr().unwrap();
    let mut wire = DatagramWire::new(Transport::Udp, 1200, TEST_MAX_PEERS);
    assert_eq!(wire.begin_input(peer, b"request"), (false, None));
    let response = Record::new(RecordKind::Handshake, 0, b"response");
    wire.send_response(&sender, peer, &response, 128)
        .await
        .unwrap();
    assert!(wire.pending.is_empty());
    wire.end_input(true);
    let mut bytes = [0; 256];
    let length = receiver.recv(&mut bytes).await.unwrap();

    let (duplicate, cached) = wire.begin_input(peer, b"request");
    assert!(duplicate);
    assert_eq!(cached.as_deref(), Some(&bytes[..length]));
}

#[test]
fn unauthenticated_udp_cache_is_bounded() {
    let mut wire = DatagramWire::new(Transport::Udp, 1200, TEST_MAX_PEERS);
    for port in 1..=u16::try_from(TEST_MAX_PEERS + 1).unwrap() {
        let peer = SocketAddr::from(([127, 0, 0, 1], port));
        let _ = wire.begin_input(peer, &port.to_be_bytes());
        wire.end_input(true);
    }
    assert_eq!(wire.seen.len(), TEST_MAX_PEERS);
}

#[test]
fn invalid_kcp_first_packet_does_not_reserve_an_engine() {
    let mut wire = DatagramWire::new(Transport::Kcp, 1200, TEST_MAX_PEERS);
    let peer = SocketAddr::from(([127, 0, 0, 1], 7000));

    assert!(wire.receive(peer, b"not a KCP packet").is_err());
    assert!(wire.engines.is_empty());
}

#[test]
fn valid_kcp_packet_requires_preflight_authorization() {
    let peer = SocketAddr::from(([127, 0, 0, 1], 7002));
    let mut client = RustKcpEngine::new_with_mtu(KCP_CONV, 1200).unwrap();
    client.send(b"complete KCP record").unwrap();
    let mut packets = Vec::new();
    client.update(0, &mut |packet| packets.push(packet.to_vec()));
    assert!(!packets.is_empty());
    let mut server = DatagramWire::new(Transport::Kcp, 1200, TEST_MAX_PEERS);

    let error = server
        .receive(peer, &packets[0])
        .expect_err("unknown KCP peers must complete preflight first");

    assert_eq!(error.code(), rnet_core::ErrorCode::AuthRejected);
    assert!(server.engines.is_empty());
}

#[tokio::test]
async fn kcp_wire_enforces_the_runtime_unacknowledged_byte_budget() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = SocketAddr::from(([127, 0, 0, 1], 7003));
    let mut wire = DatagramWire::new_with_limits(Transport::Kcp, 576, TEST_MAX_PEERS, 4096, 1000);
    wire.authorize_peer(peer).unwrap();
    let record = Record::new(RecordKind::Handshake, 0, &vec![3_u8; 400]);

    wire.send(&socket, peer, &record, 1024).await.unwrap();
    let error = wire
        .send(&socket, peer, &record, 1024)
        .await
        .expect_err("runtime KCP budget must apply across queued records");

    assert_eq!(error.code(), rnet_core::ErrorCode::WouldBlock);
    assert!(wire.kcp_queued_bytes <= wire.max_runtime_queued_bytes);
}

#[tokio::test]
async fn kcp_telemetry_is_removed_with_peer_and_endpoint() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let first = SocketAddr::from(([127, 0, 0, 1], 7010));
    let second = SocketAddr::from(([127, 0, 0, 1], 7011));
    let telemetry = Arc::new(KcpTelemetryRegistry::default());
    let mut wire = DatagramWire::new(Transport::Kcp, 1200, TEST_MAX_PEERS)
        .with_telemetry(42, Arc::clone(&telemetry));
    for peer in [first, second] {
        wire.authorize_peer(peer).unwrap();
        wire.send(
            &socket,
            peer,
            &Record::new(RecordKind::Handshake, 0, b"data"),
            1200,
        )
        .await
        .unwrap();
        assert!(telemetry.snapshot(42, peer).is_some());
    }
    wire.remove_peer(first);
    assert!(telemetry.snapshot(42, first).is_none());
    drop(wire);
    assert!(telemetry.snapshot(42, second).is_none());
}

#[test]
fn removing_a_peer_releases_every_wire_allocation() {
    let peer = SocketAddr::from(([127, 0, 0, 1], 7001));
    let mut wire = DatagramWire::new(Transport::Kcp, 1200, TEST_MAX_PEERS);
    wire.engines
        .insert(peer, super::RustKcpEngine::new(super::KCP_CONV).unwrap());
    wire.pending.insert(
        peer,
        super::PendingRecord {
            bytes: vec![1],
            last_sent: std::time::Instant::now(),
            attempts: 1,
        },
    );
    wire.seen.insert(
        peer,
        super::SeenRecord {
            request: vec![2],
            response: Some(vec![3]),
        },
    );

    wire.remove_peer(peer);

    assert!(!wire.engines.contains_key(&peer));
    assert!(!wire.pending.contains_key(&peer));
    assert!(!wire.seen.contains_key(&peer));
}
