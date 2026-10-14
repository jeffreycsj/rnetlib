use std::net::{Shutdown, SocketAddr, TcpStream as StdTcpStream};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Event, EventType, Handle, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig, SecurityMode, ServerConfig,
};

fn key() -> Keypair {
    Keypair::from_private(&[7; 32]).unwrap()
}

fn runtime_with(config: RuntimeConfig) -> NetworkRuntime {
    let key = key();
    NetworkRuntime::new_with_client_security(
        config,
        Some(ClientSecurity::pinned(key.clone(), key.public.clone())),
    )
    .unwrap()
}

fn test_runtime() -> NetworkRuntime {
    runtime_with(RuntimeConfig {
        worker_threads: 2,
        event_queue_capacity: 64,
        write_queue_capacity: 4,
        max_body_len: 1024,
        max_datagram_size: 1200,
        ..RuntimeConfig::default()
    })
}

fn listener(
    runtime: &NetworkRuntime,
    transport: Transport,
    address: &str,
) -> rnet_core::Result<Handle> {
    runtime.listen(ServerConfig {
        transport,
        bind_addr: address.parse().unwrap(),
        local_key: key(),
        initial_security: SecurityMode::Encrypted,
    })
}

fn connect(runtime: &NetworkRuntime, transport: Transport, remote_addr: SocketAddr) -> Handle {
    runtime
        .connect(ClientConfig {
            transport,
            bind_addr: None,
            remote_addr,
            join_payload: b"loopback-ticket".to_vec(),
        })
        .unwrap()
}

fn poll_until(runtime: &NetworkRuntime, mut predicate: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(10)) {
            if predicate(&event) {
                return event;
            }
        }
    }
    panic!("event did not arrive: {:?}", runtime.metrics_snapshot());
}

fn session_pair(runtime: &NetworkRuntime, server: Handle, client: Handle) -> (Handle, Handle) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let (mut server_session, mut client_session) = (None, None);
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(10)) {
            match event.event_type {
                EventType::AuthRequest => {
                    assert_eq!(&event.data[32..], b"loopback-ticket");
                    assert_eq!(
                        runtime.send(event.session, b"early").unwrap_err().code(),
                        ErrorCode::HandshakeRequired
                    );
                    runtime.auth_decide(event.session, true).unwrap();
                }
                EventType::SessionOpened if event.endpoint == server => {
                    server_session = Some(event.session)
                }
                EventType::SessionOpened if event.endpoint == client => {
                    client_session = Some(event.session)
                }
                EventType::Message => panic!("business data preceded session readiness"),
                EventType::JoinFailed => panic!("join failed: {event:?}"),
                _ => {}
            }
        }
        if let (Some(server), Some(client)) = (server_session, client_session) {
            return (server, client);
        }
    }
    panic!("sessions did not open: {:?}", runtime.metrics_snapshot());
}

fn pair(
    runtime: &NetworkRuntime,
    transport: Transport,
    bind: &str,
) -> (Handle, Handle, Handle, Handle) {
    let server = listener(runtime, transport, bind).unwrap();
    let client = connect(
        runtime,
        transport,
        runtime.endpoint_local_addr(server).unwrap(),
    );
    let (ss, cs) = session_pair(runtime, server, client);
    (server, client, ss, cs)
}

#[test]
fn authenticated_transports_exchange_opaque_messages_and_correlation() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let runtime = test_runtime();
        let (_, _, ss, cs) = pair(&runtime, transport, "127.0.0.1:0");
        for (sender, receiver, correlation, payload) in [
            (cs, ss, 41, b"from-client".as_slice()),
            (ss, cs, 42, b"from-server".as_slice()),
        ] {
            runtime
                .send_with_options(
                    sender,
                    payload,
                    rnet_transport::SendOptions {
                        correlation_id: correlation,
                    },
                )
                .unwrap();
            let event = poll_until(&runtime, |e| {
                e.event_type == EventType::Message && e.session == receiver
            });
            assert_eq!(event.data, payload);
            assert_eq!(event.correlation_id, correlation);
            assert!(event.integrity_verified);
        }
        runtime.send(cs, b"opaque").unwrap();
        let event = poll_until(&runtime, |e| {
            e.event_type == EventType::Message && e.session == ss
        });
        assert_eq!(event.data, b"opaque");
        assert_eq!(event.correlation_id, 0);
        runtime.stop(Duration::from_millis(100)).unwrap();
        runtime.stop(Duration::ZERO).unwrap();
    }
}

#[test]
fn closed_udp_route_cannot_be_reopened_by_old_business_packets() {
    let runtime = test_runtime();
    let (server, client, ss, cs) = pair(&runtime, Transport::Udp, "127.0.0.1:0");
    runtime.send(cs, b"first").unwrap();
    poll_until(&runtime, |e| {
        e.event_type == EventType::Message && e.session == ss
    });
    runtime.close_session(ss, ErrorCode::Cancelled).unwrap();
    poll_until(&runtime, |e| {
        e.event_type == EventType::SessionClosed && e.session == ss
    });
    assert_eq!(
        runtime.send(ss, b"stale").unwrap_err().code(),
        ErrorCode::InvalidHandle
    );
    runtime.send(cs, b"old-peer").unwrap();
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(10)) {
            assert!(
                !(event.endpoint == server
                    && matches!(
                        event.event_type,
                        EventType::Message | EventType::SessionOpened
                    )),
                "old packet resurrected a route: {event:?}"
            );
        }
    }
    runtime.close_endpoint(client).unwrap();
    let next = connect(
        &runtime,
        Transport::Udp,
        runtime.endpoint_local_addr(server).unwrap(),
    );
    let (next_ss, next_cs) = session_pair(&runtime, server, next);
    assert_ne!(ss, next_ss);
    runtime.send(next_cs, b"reauthorized").unwrap();
    let message = poll_until(&runtime, |e| {
        e.event_type == EventType::Message && e.session == next_ss
    });
    assert_eq!(message.data, b"reauthorized");
}

#[test]
fn oversized_message_is_rejected_before_enqueue() {
    let runtime = test_runtime();
    let (_, _, _, cs) = pair(&runtime, Transport::Udp, "127.0.0.1:0");
    assert_eq!(
        runtime.send(cs, &[0; 1025]).unwrap_err().code(),
        ErrorCode::MessageTooLarge
    );
    assert_eq!(runtime.metrics_snapshot().queued_send_bytes, 0);
}

#[test]
fn send_rejects_a_frame_that_exceeds_runtime_or_session_byte_budget() {
    let header = rnet_protocol::HEADER_LEN;
    for (global, session) in [(header, header), (1024, header)] {
        let runtime = runtime_with(RuntimeConfig {
            max_runtime_queued_bytes: global,
            max_session_queued_bytes: session,
            ..RuntimeConfig::default()
        });
        let (_, _, ss, cs) = pair(&runtime, Transport::Tcp, "127.0.0.1:0");
        assert_eq!(
            runtime.send(cs, b"x").unwrap_err().code(),
            ErrorCode::WouldBlock
        );
        assert_eq!(runtime.metrics_snapshot().queued_send_bytes, 0);
        // The header itself fits exactly; adding even one payload byte must exceed the budget.
        runtime.send(cs, b"").unwrap();
        let received = poll_until(&runtime, |event| {
            event.event_type == EventType::Message && event.session == ss
        });
        assert!(received.data.is_empty());
        // Runtime peak includes reservations rolled back after a session-budget rejection.
        // It is bounded by the global limit, not necessarily the smaller session limit.
        let peak = runtime.metrics_snapshot().peak_queued_send_bytes;
        assert!((header as u64..=global as u64).contains(&peak));
    }
}

#[test]
fn endpoint_open_reports_backpressure_and_releases_capacity() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let runtime = runtime_with(RuntimeConfig {
            event_queue_capacity: 1,
            max_endpoints: 1,
            ..RuntimeConfig::default()
        });
        assert_eq!(
            listener(&runtime, transport, "127.0.0.1:0")
                .unwrap_err()
                .code(),
            ErrorCode::WouldBlock
        );
        assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
        assert_eq!(
            runtime.poll_events(1, Duration::ZERO)[0].event_type,
            EventType::RuntimeStarted
        );
        let endpoint = listener(&runtime, transport, "127.0.0.1:0").unwrap();
        let events = runtime.poll_events(1, Duration::ZERO);
        assert_eq!(events[0].event_type, EventType::EndpointOpened);
        assert_eq!(events[0].endpoint, endpoint);
        runtime.stop(Duration::ZERO).unwrap();
    }
}

#[test]
fn udp_never_delivers_business_data_before_server_session_open() {
    // Separate queues let the client publish its ready event while the server's one-slot
    // queue still holds SessionOpened. Application traffic must not evict that lifecycle event.
    let server = runtime_with(RuntimeConfig {
        event_queue_capacity: 1,
        ..RuntimeConfig::default()
    });
    let client = test_runtime();
    server.poll_events(1, Duration::ZERO);
    let endpoint = listener(&server, Transport::Udp, "127.0.0.1:0").unwrap();
    server.poll_events(1, Duration::ZERO);
    connect(
        &client,
        Transport::Udp,
        server.endpoint_local_addr(endpoint).unwrap(),
    );
    let auth = poll_until(&server, |e| e.event_type == EventType::AuthRequest);
    server.auth_decide(auth.session, true).unwrap();
    let cs = poll_until(&client, |e| e.event_type == EventType::SessionOpened).session;
    client.send(cs, b"early").unwrap();
    let opened = poll_until(&server, |_| true);
    assert_eq!(opened.event_type, EventType::SessionOpened);
    assert_eq!(opened.session, auth.session);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            Instant::now() < deadline,
            "UDP did not recover after lifecycle backlog"
        );
        client.send(cs, b"later").unwrap();
        let mut received = false;
        for event in server.poll_events(1, Duration::from_millis(20)) {
            assert_eq!(event.event_type, EventType::Message);
            assert_eq!(event.session, opened.session);
            received |= event.data == b"later";
        }
        if received {
            break;
        }
    }
}

#[test]
fn stopped_event_does_not_replace_an_older_lifecycle_event() {
    let runtime = runtime_with(RuntimeConfig {
        event_queue_capacity: 1,
        ..RuntimeConfig::default()
    });
    runtime.poll_events(1, Duration::ZERO);
    listener(&runtime, Transport::Tcp, "127.0.0.1:0").unwrap();
    runtime.stop(Duration::ZERO).unwrap();
    assert_eq!(
        runtime.poll_events(1, Duration::ZERO)[0].event_type,
        EventType::EndpointOpened
    );
    assert_eq!(runtime.metrics_snapshot().lifecycle_events_rejected, 1);
    runtime.stop(Duration::ZERO).unwrap();
    assert!(runtime.poll_events(1, Duration::ZERO).is_empty());
}

#[test]
fn ipv6_all_transports_are_supported() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let runtime = test_runtime();
        let (_, _, ss, cs) = pair(&runtime, transport, "[::1]:0");
        runtime.send(cs, b"ipv6").unwrap();
        assert_eq!(
            poll_until(&runtime, |e| e.event_type == EventType::Message
                && e.session == ss)
            .data,
            b"ipv6"
        );
    }
}

#[test]
fn tcp_reports_port_conflict_and_unauthenticated_half_close() {
    let runtime = test_runtime();
    let endpoint = listener(&runtime, Transport::Tcp, "127.0.0.1:0").unwrap();
    let address = runtime.endpoint_local_addr(endpoint).unwrap();
    assert_eq!(
        listener(&runtime, Transport::Tcp, &address.to_string())
            .unwrap_err()
            .code(),
        ErrorCode::IoError
    );
    let stream = StdTcpStream::connect(address).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let closed = poll_until(&runtime, |e| {
        assert_ne!(
            e.event_type,
            EventType::SessionOpened,
            "raw TCP cannot become authorized"
        );
        e.event_type == EventType::SessionClosed && e.endpoint == endpoint
    });
    assert_eq!(closed.status, ErrorCode::IoError);
    assert_eq!(
        runtime
            .metrics_snapshot()
            .closed_sessions(ErrorCode::IoError),
        1
    );
    // EOF on one connection must leave the listener usable.
    let client = connect(&runtime, Transport::Tcp, address);
    session_pair(&runtime, endpoint, client);
}

#[test]
fn authenticated_tcp_listener_enforces_session_admission_limit() {
    let server = runtime_with(RuntimeConfig {
        max_sessions_per_endpoint: 1,
        ..RuntimeConfig::default()
    });
    let client = test_runtime();
    let endpoint = listener(&server, Transport::Tcp, "127.0.0.1:0").unwrap();
    let address = server.endpoint_local_addr(endpoint).unwrap();
    connect(&client, Transport::Tcp, address);
    let auth = poll_until(&server, |e| e.event_type == EventType::AuthRequest);
    server.auth_decide(auth.session, true).unwrap();
    poll_until(&server, |e| e.event_type == EventType::SessionOpened);
    poll_until(&client, |e| e.event_type == EventType::SessionOpened);
    let _second = StdTcpStream::connect(address).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.metrics_snapshot().admission_rejected == 0 {
        assert!(Instant::now() < deadline, "missing admission rejection");
        for event in server.poll_events(16, Duration::from_millis(10)) {
            assert!(!matches!(
                event.event_type,
                EventType::SessionOpened | EventType::AuthRequest
            ));
        }
    }
    assert_eq!(server.metrics_snapshot().current_sessions, 1);
}

#[test]
fn runtime_rejects_zero_session_capacity() {
    let error = NetworkRuntime::new(RuntimeConfig {
        max_sessions_per_endpoint: 0,
        ..RuntimeConfig::default()
    })
    .err()
    .expect("zero session capacity must be rejected");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn runtime_rejects_zero_byte_budgets() {
    for config in [
        RuntimeConfig {
            max_event_bytes: 0,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            max_runtime_queued_bytes: 0,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            max_session_queued_bytes: 0,
            ..RuntimeConfig::default()
        },
    ] {
        assert_eq!(
            NetworkRuntime::new(config)
                .err()
                .expect("zero byte budget must be rejected")
                .code(),
            ErrorCode::InvalidArgument
        );
    }
}

#[test]
fn runtime_rejects_explicit_zero_tcp_socket_buffers() {
    let error = NetworkRuntime::new(RuntimeConfig {
        tcp_send_buffer_bytes: Some(0),
        ..RuntimeConfig::default()
    })
    .err()
    .expect("an explicit zero TCP buffer must be rejected");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn runtime_rejects_timeouts_that_cannot_form_a_deadline() {
    for config in [
        RuntimeConfig {
            handshake_timeout: Duration::MAX,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            connect_timeout: Duration::MAX,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            dns_timeout: Duration::MAX,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            datagram_idle_timeout: Duration::MAX,
            ..RuntimeConfig::default()
        },
        RuntimeConfig {
            security_policy: rnet_transport::SecurityPolicy {
                rekey_after: Some(Duration::MAX),
                ..rnet_transport::SecurityPolicy::production()
            },
            ..RuntimeConfig::default()
        },
    ] {
        let error = NetworkRuntime::new(config)
            .err()
            .expect("unrepresentable timeout must be rejected");
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }
}

#[test]
fn stop_rejects_an_unrepresentable_deadline_before_draining() {
    let runtime = test_runtime();

    let error = runtime
        .stop(Duration::MAX)
        .expect_err("unrepresentable drain timeout must be rejected");

    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn stop_without_sessions_does_not_wait_for_the_full_drain_timeout() {
    let runtime = test_runtime();
    let started = Instant::now();

    runtime.stop(Duration::from_secs(1)).unwrap();

    assert!(started.elapsed() < Duration::from_millis(100));
}

#[test]
fn prometheus_snapshot_exports_resource_and_p999_metrics() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).unwrap();

    let text = runtime.prometheus_snapshot();

    assert!(text.contains("rnet_queued_send_bytes 0"));
    assert!(text.contains("quantile=\"0.999\""));
    assert!(text.contains("rnet_admission_rejected_total"));
}
