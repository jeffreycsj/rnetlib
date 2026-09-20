use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Event, EventType, Handle, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientConfig, ClientSecurity, HostClientConfig, LatencyKind, NetworkRuntime,
    ResolvedClientConfig, RuntimeConfig, SecurityChange, SecurityMode, SecurityOperation,
    ServerConfig,
};

fn localhost(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn poll_until(runtime: &NetworkRuntime, event_type: EventType) -> Event {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(event) = runtime
            .poll_events(32, Duration::from_millis(20))
            .into_iter()
            .find(|event| event.event_type == event_type)
        {
            return event;
        }
    }
    panic!(
        "timed out waiting for {event_type:?}, metrics={:?}",
        runtime.metrics_snapshot()
    );
}

fn assert_message(runtime: &NetworkRuntime, expected: &[u8], stage: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut event = None;
    while Instant::now() < deadline && event.is_none() {
        event = runtime
            .poll_events(32, Duration::from_millis(20))
            .into_iter()
            .find(|candidate| candidate.event_type == EventType::Message);
    }
    let event = event.unwrap_or_else(|| {
        panic!(
            "{stage} timed out, metrics={:?}",
            runtime.metrics_snapshot()
        )
    });
    assert_eq!(event.data, expected);
}

fn establish_pair(
    transport: Transport,
    initial_security: SecurityMode,
) -> (NetworkRuntime, Handle, Handle, Handle) {
    establish_pair_with_config(transport, initial_security, RuntimeConfig::default())
}

fn establish_pair_with_config(
    transport: Transport,
    initial_security: SecurityMode,
    config: RuntimeConfig,
) -> (NetworkRuntime, Handle, Handle, Handle) {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime =
        NetworkRuntime::new_with_client_security(config, Some(client_security)).expect("runtime");
    let listener = runtime
        .listen(ServerConfig {
            transport,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_security,
        })
        .expect("listener");
    let client = runtime
        .connect_host(HostClientConfig {
            transport,
            host: "localhost".to_owned(),
            port: runtime.endpoint_local_addr(listener).unwrap().port(),
            join_payload: b"ticket".to_vec(),
        })
        .expect("client");
    let auth = poll_until(&runtime, EventType::AuthRequest);
    runtime
        .auth_decide(auth.session, true)
        .expect("accept client");

    let mut server_session = 0;
    let mut client_session = 0;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && (server_session == 0 || client_session == 0) {
        for event in runtime.poll_events(32, Duration::from_millis(20)) {
            if event.event_type == EventType::SessionOpened {
                if event.endpoint == listener {
                    server_session = event.session;
                } else if event.endpoint == client {
                    client_session = event.session;
                }
            }
        }
    }
    assert_ne!(server_session, 0);
    assert_ne!(client_session, 0);
    (runtime, listener, server_session, client_session)
}

fn assert_transport_follows_server_security_changes(transport: Transport) {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime =
        NetworkRuntime::new_with_client_security(RuntimeConfig::default(), Some(client_security))
            .expect("runtime");

    let listener = runtime
        .listen(ServerConfig {
            transport,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_security: SecurityMode::Plaintext,
        })
        .expect("listener");
    let client = runtime
        .connect(ClientConfig {
            transport,
            bind_addr: (transport != Transport::Tcp).then(|| localhost(0)),
            remote_addr: localhost(runtime.endpoint_local_addr(listener).unwrap().port()),
            join_payload: b"ticket".to_vec(),
        })
        .expect("client");

    let auth = poll_until(&runtime, EventType::AuthRequest);
    runtime
        .auth_decide(auth.session, true)
        .expect("accept client");
    let mut server_session = 0;
    let mut client_session = 0;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && (server_session == 0 || client_session == 0) {
        for event in runtime.poll_events(32, Duration::from_millis(20)) {
            if event.event_type == EventType::SessionOpened {
                if event.endpoint == listener {
                    server_session = event.session;
                } else if event.endpoint == client {
                    client_session = event.session;
                }
            }
        }
    }
    assert_ne!(server_session, 0);
    assert_ne!(client_session, 0);

    runtime.send(client_session, 1, b"plain").unwrap();
    assert_message(&runtime, b"plain", "plain");

    runtime
        .set_security_mode(server_session, SecurityMode::Encrypted)
        .expect("request encryption");
    let changed = poll_until(&runtime, EventType::SecurityChanged);
    assert_eq!(
        SecurityChange::from_event(&changed).unwrap().operation,
        SecurityOperation::ModeSwitch
    );
    runtime.send(client_session, 1, b"encrypted").unwrap();
    assert_message(&runtime, b"encrypted", "encrypted");

    runtime
        .rekey_session(server_session)
        .expect("request rekey");
    let changed = poll_until(&runtime, EventType::SecurityChanged);
    let change = SecurityChange::from_event(&changed).unwrap();
    assert_eq!(change.operation, SecurityOperation::Rekey);
    assert_eq!(change.mode, SecurityMode::Encrypted);
    assert!(change.epoch >= 3);
    runtime.send(client_session, 1, b"rekeyed").unwrap();
    assert_message(&runtime, b"rekeyed", "rekeyed");

    runtime
        .set_security_mode(server_session, SecurityMode::Plaintext)
        .expect("request plaintext");
    poll_until(&runtime, EventType::SecurityChanged);
    runtime.send(client_session, 1, b"plain-again").unwrap();
    assert_message(&runtime, b"plain-again", "plain-again");
}

#[test]
fn clients_automatically_follow_server_security_changes() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        assert_transport_follows_server_security_changes(transport);
    }
}

#[test]
fn server_automatically_rekeys_after_the_encrypted_byte_threshold() {
    let mut config = RuntimeConfig::default();
    config.security_policy.rekey_after = None;
    config.security_policy.rekey_after_bytes = Some(4);
    let (runtime, _, server_session, _) =
        establish_pair_with_config(Transport::Tcp, SecurityMode::Encrypted, config);

    runtime.send(server_session, 1, b"four").unwrap();
    assert_message(&runtime, b"four", "automatic rekey trigger message");
    let changed = poll_until(&runtime, EventType::SecurityChanged);
    let change = SecurityChange::from_event(&changed).unwrap();

    assert_eq!(change.operation, SecurityOperation::Rekey);
    assert!(change.epoch >= 2);
}

#[test]
fn datagram_servers_automatically_rekey_after_the_encrypted_byte_threshold() {
    for transport in [Transport::Udp, Transport::Kcp] {
        let mut config = RuntimeConfig::default();
        config.security_policy.rekey_after = None;
        config.security_policy.rekey_after_bytes = Some(4);
        let (runtime, _, server_session, _) =
            establish_pair_with_config(transport, SecurityMode::Encrypted, config);

        runtime.send(server_session, 1, b"four").unwrap();
        assert_message(&runtime, b"four", "datagram automatic rekey trigger");
        let changed = poll_until(&runtime, EventType::SecurityChanged);
        assert_eq!(
            SecurityChange::from_event(&changed).unwrap().operation,
            SecurityOperation::Rekey
        );
    }
}

#[test]
fn adaptive_udp_rejects_payload_that_exceeds_the_encrypted_wire_mtu() {
    let (runtime, _, _, client_session) = establish_pair(Transport::Udp, SecurityMode::Encrypted);
    let payload = vec![7; 1_150];

    let error = runtime
        .send(client_session, 7, &payload)
        .expect_err("wire-overhead overflow must be rejected before enqueue");

    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
}

#[test]
fn adaptive_kcp_fragments_messages_larger_than_one_datagram() {
    let (runtime, _, _, client_session) = establish_pair(Transport::Kcp, SecurityMode::Encrypted);
    let payload = vec![0x5a; 16 * 1024];

    runtime
        .send(client_session, 8, &payload)
        .expect("KCP must accept a message within max_body_len");
    assert_message(&runtime, &payload, "large KCP message");
}

#[test]
fn adaptive_udp_enforces_the_per_ip_active_session_limit() {
    let server_key = Keypair::generate().expect("server key");
    let client_security = ClientSecurity::pinned(
        Keypair::generate().expect("client key"),
        server_key.public.clone(),
    );
    let config = RuntimeConfig {
        max_sessions_per_ip: 1,
        ..RuntimeConfig::default()
    };
    let runtime =
        NetworkRuntime::new_with_client_security(config, Some(client_security)).expect("runtime");
    let listener = runtime
        .listen(ServerConfig {
            transport: Transport::Udp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_security: SecurityMode::Encrypted,
        })
        .expect("listener");
    let remote_addr = localhost(runtime.endpoint_local_addr(listener).unwrap().port());
    let first = runtime
        .connect(ClientConfig {
            transport: Transport::Udp,
            bind_addr: Some(localhost(0)),
            remote_addr,
            join_payload: Vec::new(),
        })
        .expect("first client");
    let auth = poll_until(&runtime, EventType::AuthRequest);
    runtime.auth_decide(auth.session, true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if runtime
            .poll_events(32, Duration::from_millis(20))
            .iter()
            .any(|event| event.event_type == EventType::SessionOpened && event.endpoint == first)
        {
            break;
        }
    }

    runtime
        .connect(ClientConfig {
            transport: Transport::Udp,
            bind_addr: Some(localhost(0)),
            remote_addr,
            join_payload: Vec::new(),
        })
        .expect("second client endpoint");
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut saw_second_auth = false;
    while Instant::now() < deadline {
        saw_second_auth |= runtime
            .poll_events(32, Duration::from_millis(20))
            .iter()
            .any(|event| event.event_type == EventType::AuthRequest);
    }

    assert!(!saw_second_auth);
    assert!(runtime.metrics_snapshot().admission_rejected > 0);
}

#[test]
fn udp_client_retries_the_next_resolved_address_after_handshake_timeout() {
    assert_datagram_address_failover(Transport::Udp);
}

#[test]
fn kcp_client_retries_the_next_resolved_address_after_handshake_timeout() {
    assert_datagram_address_failover(Transport::Kcp);
}

fn assert_datagram_address_failover(transport: Transport) {
    let server_key = Keypair::generate().unwrap();
    let client_security =
        ClientSecurity::pinned(Keypair::generate().unwrap(), server_key.public.clone());
    let config = RuntimeConfig {
        handshake_timeout: Duration::from_millis(500),
        ..RuntimeConfig::default()
    };
    let runtime =
        NetworkRuntime::new_with_client_security(config, Some(client_security)).expect("runtime");
    let listener = runtime
        .listen(ServerConfig {
            transport,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_security: SecurityMode::Encrypted,
        })
        .unwrap();
    let blackhole = std::net::UdpSocket::bind(localhost(0)).unwrap();
    let client = runtime
        .connect_resolved(ResolvedClientConfig {
            transport,
            remote_addrs: vec![
                blackhole.local_addr().unwrap(),
                runtime.endpoint_local_addr(listener).unwrap(),
            ],
            join_payload: b"retry-ticket".to_vec(),
        })
        .unwrap();

    let auth = poll_until(&runtime, EventType::AuthRequest);
    runtime.auth_decide(auth.session, true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut client_opened = false;
    while Instant::now() < deadline && !client_opened {
        client_opened = runtime
            .poll_events(32, Duration::from_millis(20))
            .iter()
            .any(|event| event.event_type == EventType::SessionOpened && event.endpoint == client);
    }
    assert!(
        client_opened,
        "{transport:?} did not open after address failover"
    );
}

#[test]
fn adaptive_client_rejects_oversized_join_payload_before_opening_endpoint() {
    let server_key = Keypair::generate().expect("server key");
    let client_security = ClientSecurity::pinned(
        Keypair::generate().expect("client key"),
        server_key.public.clone(),
    );
    let runtime =
        NetworkRuntime::new_with_client_security(RuntimeConfig::default(), Some(client_security))
            .expect("runtime");
    let listener = runtime
        .listen(ServerConfig {
            transport: Transport::Udp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_security: SecurityMode::Encrypted,
        })
        .expect("listener");

    let error = runtime
        .connect(ClientConfig {
            transport: Transport::Udp,
            bind_addr: Some(localhost(0)),
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_payload: vec![0; 60 * 1024 + 1],
        })
        .expect_err("oversized join data must fail synchronously");

    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
}

#[test]
fn adaptive_kcp_populates_enterprise_latency_and_traffic_metrics() {
    let (runtime, _, _, client_session) = establish_pair(Transport::Kcp, SecurityMode::Encrypted);
    runtime.send(client_session, 9, b"metric-sample").unwrap();
    assert_message(&runtime, b"metric-sample", "metric sample");

    let snapshots = runtime.latency_snapshot();
    for kind in [
        LatencyKind::Connect,
        LatencyKind::CryptoHandshake,
        LatencyKind::AuthWait,
        LatencyKind::SendQueue,
        LatencyKind::KcpUpdateDelay,
    ] {
        let sample = snapshots
            .iter()
            .find(|snapshot| snapshot.kind == kind)
            .expect("latency kind");
        assert!(sample.latency.sample_count > 0, "missing {kind:?} sample");
    }
    assert!(runtime.metrics_snapshot().bytes_sent >= b"metric-sample".len() as u64);
}

#[test]
fn adaptive_tcp_records_application_auth_wait_latency() {
    let (runtime, _, _, _) = establish_pair(Transport::Tcp, SecurityMode::Encrypted);
    let auth_wait = runtime
        .latency_snapshot()
        .into_iter()
        .find(|metric| metric.kind == LatencyKind::AuthWait)
        .expect("auth wait metric");

    assert!(auth_wait.latency.sample_count >= 2);
}

#[test]
fn adaptive_tcp_closes_a_client_that_never_sends_client_hello() {
    let server_key = Keypair::generate().expect("server key");
    let runtime = NetworkRuntime::new(RuntimeConfig {
        handshake_timeout: Duration::from_millis(100),
        ..RuntimeConfig::default()
    })
    .expect("runtime");
    let listener = runtime
        .listen(ServerConfig {
            transport: Transport::Tcp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_security: SecurityMode::Encrypted,
        })
        .expect("listener");
    let _silent_client =
        std::net::TcpStream::connect(runtime.endpoint_local_addr(listener).unwrap()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut closed = None;
    while Instant::now() < deadline && closed.is_none() {
        closed = runtime
            .poll_events(16, Duration::from_millis(50))
            .into_iter()
            .find(|event| event.event_type == EventType::SessionClosed);
    }

    assert_eq!(
        closed
            .expect("silent TCP handshake was not reclaimed")
            .status,
        ErrorCode::Timeout
    );
}

#[test]
fn adaptive_udp_reclaims_an_idle_established_peer() {
    let (runtime, listener, server_session, _) = establish_pair_with_config(
        Transport::Udp,
        SecurityMode::Encrypted,
        RuntimeConfig {
            datagram_idle_timeout: Duration::from_millis(50),
            ..RuntimeConfig::default()
        },
    );

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut closed = None;
    while Instant::now() < deadline && closed.is_none() {
        closed = runtime
            .poll_events(16, Duration::from_millis(20))
            .into_iter()
            .find(|event| {
                event.event_type == EventType::SessionClosed
                    && event.endpoint == listener
                    && event.session == server_session
            });
    }

    assert_eq!(
        closed.expect("idle UDP peer was not reclaimed").status,
        ErrorCode::Timeout
    );
}
