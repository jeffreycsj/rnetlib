use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Event, EventType, Handle};
use rnet_security::{InitiatorHandshake, Keypair};
use rnet_transport::{EndpointConfig, LatencyKind, NetworkRuntime, RuntimeConfig};

fn poll_until(
    runtime: &NetworkRuntime,
    timeout: Duration,
    mut predicate: impl FnMut(&Event) -> bool,
) -> Event {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(20)) {
            if predicate(&event) {
                return event;
            }
        }
    }
    panic!("event did not arrive before timeout");
}

fn poll_open_pair(
    runtime: &NetworkRuntime,
    timeout: Duration,
    listener: Handle,
    client: Handle,
) -> (Handle, Handle) {
    let deadline = Instant::now() + timeout;
    let mut server_open = None;
    let mut client_open = None;
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(20)) {
            if event.event_type == EventType::SessionOpened && event.endpoint == listener {
                server_open = Some(event.session);
            } else if event.event_type == EventType::SessionOpened && event.endpoint == client {
                client_open = Some(event.session);
            } else if matches!(
                event.event_type,
                EventType::JoinFailed | EventType::SessionClosed
            ) {
                panic!("session failed before both sides opened: {event:?}");
            }
        }
        if let (Some(server), Some(client)) = (server_open, client_open) {
            return (server, client);
        }
    }
    panic!(
        "both open events did not arrive: server={}, client={}, metrics={:?}",
        server_open.is_some(),
        client_open.is_some(),
        runtime.metrics_snapshot()
    );
}

#[test]
fn secure_tcp_requires_server_auth_before_opening_and_exchanging_frames() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).expect("runtime");
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let listener = runtime
        .open_endpoint(EndpointConfig::secure_tcp_listener(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            server_key.clone(),
        ))
        .expect("secure listener");
    let server_addr = runtime
        .endpoint_local_addr(listener)
        .expect("listener addr");
    let client = runtime
        .open_endpoint(EndpointConfig::secure_tcp_client(
            server_addr,
            client_key.clone(),
            server_key.public.clone(),
            b"account-ticket".to_vec(),
        ))
        .expect("secure client");

    let auth = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::AuthRequest && event.endpoint == listener
    });
    assert_eq!(&auth.data[..32], client_key.public.as_slice());
    assert_eq!(&auth.data[32..], b"account-ticket");
    assert_eq!(
        runtime
            .send(auth.session, 1, b"too-early")
            .unwrap_err()
            .code(),
        ErrorCode::HandshakeRequired
    );
    runtime
        .auth_decide(auth.session, true)
        .expect("accept auth");

    // Give both sides time to enqueue before polling, exposing lost-event bugs in batch consumers.
    std::thread::sleep(Duration::from_millis(100));

    let (server_session, client_session) =
        poll_open_pair(&runtime, Duration::from_secs(2), listener, client);
    assert_eq!(
        runtime
            .send_game_control(client_session, b"reserved")
            .expect_err("legacy secure sessions cannot send game controls")
            .code(),
        ErrorCode::NotSupported
    );
    runtime
        .send_legacy(client_session, 7, 1, 99, b"encrypted-frame")
        .expect("send encrypted frame");
    let message = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_session
    });
    assert_eq!(message.data, b"encrypted-frame");
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_tcp_rejects_an_unpinned_server() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).expect("runtime");
    let server_key = Keypair::generate().expect("server key");
    let wrong_key = Keypair::generate().expect("wrong key");
    let client_key = Keypair::generate().expect("client key");
    let listener = runtime
        .open_endpoint(EndpointConfig::secure_tcp_listener(
            "127.0.0.1:0".parse().unwrap(),
            server_key,
        ))
        .expect("secure listener");
    let client = runtime
        .open_endpoint(EndpointConfig::secure_tcp_client(
            runtime.endpoint_local_addr(listener).unwrap(),
            client_key,
            wrong_key.public.clone(),
            Vec::new(),
        ))
        .expect("secure client");
    let failed = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::JoinFailed && event.endpoint == client
    });
    assert_eq!(failed.status, ErrorCode::PeerKeyMismatch);
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_udp_establishes_a_logical_session_before_delivering_datagrams() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).expect("runtime");
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let listener = runtime
        .open_endpoint(EndpointConfig::secure_udp_listener(
            "127.0.0.1:0".parse().unwrap(),
            server_key.clone(),
        ))
        .expect("UDP listener");
    let client = runtime
        .open_endpoint(EndpointConfig::secure_udp_client(
            "127.0.0.1:0".parse().unwrap(),
            runtime.endpoint_local_addr(listener).unwrap(),
            client_key,
            server_key.public.clone(),
            b"udp-ticket".to_vec(),
        ))
        .expect("UDP client");
    let auth = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::AuthRequest && event.endpoint == listener
    });
    runtime.auth_decide(auth.session, true).expect("accept UDP");
    let (server_session, client_session) =
        poll_open_pair(&runtime, Duration::from_secs(2), listener, client);
    runtime
        .send_with_options(
            client_session,
            5,
            b"secure-datagram",
            rnet_transport::SendOptions { correlation_id: 1 },
        )
        .expect("send UDP");
    let message = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_session
    });
    assert_eq!(message.data, b"secure-datagram");
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_kcp_establishes_and_exchanges_a_reliable_message() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).expect("runtime");
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let listener = runtime
        .open_endpoint(EndpointConfig::secure_kcp_listener(
            "127.0.0.1:0".parse().unwrap(),
            server_key.clone(),
        ))
        .expect("KCP listener");
    let client = runtime
        .open_endpoint(EndpointConfig::secure_kcp_client(
            "127.0.0.1:0".parse().unwrap(),
            runtime.endpoint_local_addr(listener).unwrap(),
            client_key,
            server_key.public.clone(),
            b"kcp-ticket".to_vec(),
        ))
        .expect("KCP client");
    let auth = poll_until(&runtime, Duration::from_secs(3), |event| {
        event.event_type == EventType::AuthRequest && event.endpoint == listener
    });
    runtime.auth_decide(auth.session, true).expect("accept KCP");
    let (server_session, client_session) =
        poll_open_pair(&runtime, Duration::from_secs(3), listener, client);
    let payload = vec![91_u8; 4096];
    runtime
        .send_with_options(
            client_session,
            8,
            &payload,
            rnet_transport::SendOptions { correlation_id: 2 },
        )
        .expect("send KCP");
    let message = poll_until(&runtime, Duration::from_secs(3), |event| {
        event.event_type == EventType::Message && event.session == server_session
    });
    assert_eq!(message.data, payload);
    let latencies = runtime.latency_snapshot();
    for kind in [LatencyKind::CryptoHandshake, LatencyKind::AuthWait] {
        assert!(
            latencies
                .iter()
                .find(|metric| metric.kind == kind)
                .expect("secure session latency metric")
                .latency
                .sample_count
                > 0
        );
    }
    assert!(
        latencies
            .iter()
            .find(|metric| metric.kind == LatencyKind::KcpRtt)
            .expect("KCP RTT metric")
            .latency
            .sample_count
            > 0
    );
    assert!(
        latencies
            .iter()
            .find(|metric| metric.kind == LatencyKind::KcpUpdateDelay)
            .expect("KCP update delay metric")
            .latency
            .sample_count
            > 0
    );
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_udp_requires_a_stateless_cookie_before_noise_allocation() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).expect("runtime");
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let listener = runtime
        .open_endpoint(EndpointConfig::secure_udp_listener(
            "127.0.0.1:0".parse().unwrap(),
            server_key.clone(),
        ))
        .expect("listener");
    let mut handshake =
        InitiatorHandshake::new(&client_key.private, &server_key.public).expect("initiator");
    let first = handshake.write_first().expect("first Noise message");
    let mut packet = vec![1];
    packet.extend_from_slice(&first);
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("raw UDP client");
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    socket
        .send_to(&packet, runtime.endpoint_local_addr(listener).unwrap())
        .expect("send first");
    let mut response = [0_u8; 128];
    let (length, _) = socket.recv_from(&mut response).expect("cookie response");
    assert!(length > 1);
    assert_eq!(
        response[0], 6,
        "server must challenge before Noise response"
    );
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn oversized_join_payload_is_rejected_before_connecting() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).expect("runtime");
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let error = runtime
        .open_endpoint(EndpointConfig::secure_tcp_client(
            "127.0.0.1:9".parse().unwrap(),
            client_key,
            server_key.public.clone(),
            vec![0; 60 * 1024 + 1],
        ))
        .expect_err("oversized join must fail before connect");
    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
    runtime.stop(Duration::ZERO).expect("stop");
}
