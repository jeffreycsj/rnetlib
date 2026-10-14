use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Event, EventType, Handle, Transport};
use rnet_protocol::control::{decode_record, encode_record, Record, RecordKind};
use rnet_security::Keypair;
use rnet_transport::{
    ClientConfig, ClientSecurity, LatencyKind, NetworkRuntime, RuntimeConfig, SecurityMode,
    ServerConfig,
};

fn authenticated_runtime(client_key: Keypair, server_public: Vec<u8>) -> NetworkRuntime {
    NetworkRuntime::new_with_client_security(
        RuntimeConfig::default(),
        Some(ClientSecurity::pinned(client_key, server_public)),
    )
    .expect("runtime")
}

fn listen(runtime: &NetworkRuntime, transport: Transport, key: Keypair) -> Handle {
    runtime
        .listen(ServerConfig {
            transport,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: key,
            initial_security: SecurityMode::Encrypted,
        })
        .expect("listener")
}

fn connect(
    runtime: &NetworkRuntime,
    transport: Transport,
    remote_addr: SocketAddr,
    ticket: &[u8],
) -> Handle {
    runtime
        .connect(ClientConfig {
            transport,
            bind_addr: None,
            remote_addr,
            join_payload: ticket.to_vec(),
        })
        .expect("client")
}

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
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = authenticated_runtime(client_key.clone(), server_key.public.clone());
    let listener = listen(&runtime, Transport::Tcp, server_key);
    let server_addr = runtime
        .endpoint_local_addr(listener)
        .expect("listener addr");
    let client = connect(&runtime, Transport::Tcp, server_addr, b"account-ticket");

    let auth = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::AuthRequest && event.endpoint == listener
    });
    assert_eq!(&auth.data[..32], client_key.public.as_slice());
    assert_eq!(&auth.data[32..], b"account-ticket");
    assert_eq!(
        runtime.send(auth.session, b"too-early").unwrap_err().code(),
        ErrorCode::HandshakeRequired
    );
    runtime
        .auth_decide(auth.session, true)
        .expect("accept auth");

    // Give both sides time to enqueue before polling, exposing lost-event bugs in batch consumers.
    std::thread::sleep(Duration::from_millis(100));

    let (server_session, client_session) =
        poll_open_pair(&runtime, Duration::from_secs(2), listener, client);
    runtime
        .send_with_options(
            client_session,
            b"encrypted-frame",
            rnet_transport::SendOptions { correlation_id: 99 },
        )
        .expect("send encrypted frame");
    let message = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_session
    });
    assert_eq!(message.data, b"encrypted-frame");
    assert_eq!(message.request_id, 99);
    assert!(message.integrity_verified);
    runtime
        .send_game_control(client_session, b"reserved")
        .unwrap();
    let control = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::GameControl && event.session == server_session
    });
    assert_eq!(control.data, b"reserved");
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_tcp_rejects_an_unpinned_server() {
    let server_key = Keypair::generate().expect("server key");
    let wrong_key = Keypair::generate().expect("wrong key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = authenticated_runtime(client_key, wrong_key.public.clone());
    let listener = listen(&runtime, Transport::Tcp, server_key);
    let client = connect(
        &runtime,
        Transport::Tcp,
        runtime.endpoint_local_addr(listener).unwrap(),
        b"",
    );
    let failed = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::JoinFailed && event.endpoint == client
    });
    assert_eq!(failed.status, ErrorCode::PeerKeyMismatch);
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_udp_establishes_a_logical_session_before_delivering_datagrams() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = authenticated_runtime(client_key, server_key.public.clone());
    let listener = listen(&runtime, Transport::Udp, server_key);
    let client = connect(
        &runtime,
        Transport::Udp,
        runtime.endpoint_local_addr(listener).unwrap(),
        b"udp-ticket",
    );
    let auth = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::AuthRequest && event.endpoint == listener
    });
    runtime.auth_decide(auth.session, true).expect("accept UDP");
    let (server_session, client_session) =
        poll_open_pair(&runtime, Duration::from_secs(2), listener, client);
    runtime
        .send_with_options(
            client_session,
            b"secure-datagram",
            rnet_transport::SendOptions { correlation_id: 1 },
        )
        .expect("send UDP");
    let message = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_session
    });
    assert_eq!(message.data, b"secure-datagram");
    assert_eq!(message.request_id, 1);
    assert!(message.integrity_verified);
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn secure_kcp_establishes_and_exchanges_a_reliable_message() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = authenticated_runtime(client_key, server_key.public.clone());
    let listener = listen(&runtime, Transport::Kcp, server_key);
    let client = connect(
        &runtime,
        Transport::Kcp,
        runtime.endpoint_local_addr(listener).unwrap(),
        b"kcp-ticket",
    );
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
            &payload,
            rnet_transport::SendOptions { correlation_id: 2 },
        )
        .expect("send KCP");
    let message = poll_until(&runtime, Duration::from_secs(3), |event| {
        event.event_type == EventType::Message && event.session == server_session
    });
    assert_eq!(message.data, payload);
    assert_eq!(message.request_id, 2);
    assert!(message.integrity_verified);
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
    let listener = listen(&runtime, Transport::Udp, server_key);
    let packet = encode_record(&Record::new(RecordKind::ClientHello, 0, b""), 128).unwrap();
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("raw UDP client");
    socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    socket
        .send_to(&packet, runtime.endpoint_local_addr(listener).unwrap())
        .expect("send first");
    let mut response = [0_u8; 128];
    let (length, _) = socket.recv_from(&mut response).expect("cookie response");
    let response = decode_record(&response[..length], 128).unwrap();
    assert_eq!(
        response.kind,
        RecordKind::CookieChallenge,
        "server must challenge before Noise response"
    );
    assert_eq!(runtime.metrics_snapshot().current_sessions, 0);
    assert_eq!(runtime.metrics_snapshot().pending_handshakes, 0);
    runtime.stop(Duration::ZERO).expect("stop");
}

#[test]
fn oversized_join_payload_is_rejected_before_connecting() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = authenticated_runtime(client_key, server_key.public.clone());
    let error = runtime
        .connect(ClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: "127.0.0.1:9".parse().unwrap(),
            join_payload: vec![0; 60 * 1024 + 1],
        })
        .expect_err("oversized join must fail before connect");
    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
    assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
    runtime.stop(Duration::ZERO).expect("stop");
}
