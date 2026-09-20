use std::net::{Shutdown, SocketAddr, TcpStream as StdTcpStream};
use std::time::{Duration, Instant};

use rnet_core::{ErrorCode, Event, EventType};
use rnet_transport::{EndpointConfig, NetworkRuntime, RuntimeConfig};

fn test_runtime() -> NetworkRuntime {
    NetworkRuntime::new(RuntimeConfig {
        worker_threads: 2,
        event_queue_capacity: 64,
        write_queue_capacity: 4,
        max_body_len: 1024,
        max_datagram_size: 1200,
        ..RuntimeConfig::default()
    })
    .unwrap()
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
fn production_config_rejects_legacy_unauthenticated_endpoints() {
    let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
    let error = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotSupported);
}

#[test]
fn stop_without_sessions_does_not_wait_for_the_full_drain_timeout() {
    let runtime = test_runtime();
    let started = Instant::now();

    runtime.stop(Duration::from_secs(1)).unwrap();

    assert!(started.elapsed() < Duration::from_millis(100));
}

fn poll_until(
    runtime: &NetworkRuntime,
    timeout: Duration,
    mut predicate: impl FnMut(&Event) -> bool,
) -> Event {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(50)) {
            if predicate(&event) {
                return event;
            }
        }
    }
    panic!("event did not arrive before timeout");
}

fn poll_session_pair(
    runtime: &NetworkRuntime,
    first_endpoint: u64,
    second_endpoint: u64,
) -> (Event, Event) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut first = None;
    let mut second = None;
    while Instant::now() < deadline && (first.is_none() || second.is_none()) {
        for event in runtime.poll_events(16, Duration::from_millis(50)) {
            if event.event_type != EventType::SessionOpened {
                continue;
            }
            if event.endpoint == first_endpoint {
                first = Some(event);
            } else if event.endpoint == second_endpoint {
                second = Some(event);
            }
        }
    }
    (
        first.expect("first session did not open"),
        second.expect("second session did not open"),
    )
}

#[test]
fn tcp_listener_and_client_exchange_framed_messages() {
    let runtime = test_runtime();
    let listener = runtime
        .open_endpoint(EndpointConfig::tcp_listener(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        ))
        .unwrap();
    let address = runtime.endpoint_local_addr(listener).unwrap();
    let client = runtime
        .open_endpoint(EndpointConfig::tcp_client(address))
        .unwrap();

    let (client_open, server_open) = poll_session_pair(&runtime, client, listener);

    runtime
        .send_legacy(client_open.session, 7, 2, 41, b"from-client")
        .unwrap();
    let request = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_open.session
    });
    assert_eq!(request.msg_type, 7);
    assert_eq!(request.stream_id, 2);
    assert_eq!(request.request_id, 41);
    assert_eq!(request.data, b"from-client");

    runtime
        .send_legacy(server_open.session, 8, 2, 42, b"from-server")
        .unwrap();
    let response = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == client_open.session
    });
    assert_eq!(response.data, b"from-server");

    runtime
        .send_payload(client_open.session, b"type-owned-by-payload")
        .unwrap();
    let payload_only = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_open.session
    });
    assert_eq!(payload_only.msg_type, 0);
    assert_eq!(payload_only.stream_id, 0);
    assert_eq!(payload_only.request_id, 0);
    assert_eq!(payload_only.data, b"type-owned-by-payload");
    runtime.stop(Duration::from_millis(100)).unwrap();
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn udp_endpoints_exchange_one_frame_per_datagram() {
    let runtime = test_runtime();
    let server = runtime
        .open_endpoint(EndpointConfig::udp("127.0.0.1:0".parse().unwrap(), None))
        .unwrap();
    let server_address = runtime.endpoint_local_addr(server).unwrap();
    let client = runtime
        .open_endpoint(EndpointConfig::udp(
            "127.0.0.1:0".parse().unwrap(),
            Some(server_address),
        ))
        .unwrap();
    let client_open = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == client
    });

    runtime
        .send_legacy(client_open.session, 9, 0, 77, b"datagram")
        .unwrap();
    let server_message = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.endpoint == server
    });
    assert_eq!(server_message.data, b"datagram");

    runtime
        .send_legacy(server_message.session, 10, 0, 78, b"reply")
        .unwrap();
    let reply = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == client_open.session
    });
    assert_eq!(reply.data, b"reply");
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn closing_a_udp_session_never_reuses_its_stale_peer_route() {
    let runtime = test_runtime();
    let server = runtime
        .open_endpoint(EndpointConfig::udp("127.0.0.1:0".parse().unwrap(), None))
        .unwrap();
    let client = runtime
        .open_endpoint(EndpointConfig::udp(
            "127.0.0.1:0".parse().unwrap(),
            Some(runtime.endpoint_local_addr(server).unwrap()),
        ))
        .unwrap();
    let client_session = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == client
    })
    .session;
    runtime.send(client_session, 1, b"first").unwrap();
    let first = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.endpoint == server
    });

    runtime
        .close_session(first.session, ErrorCode::Cancelled)
        .unwrap();
    let closed = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionClosed && event.session == first.session
    });
    assert_eq!(closed.status, ErrorCode::Cancelled);

    runtime.send(client_session, 2, b"second").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut reopened_session = None;
    let mut message_session = None;
    while Instant::now() < deadline && message_session.is_none() {
        for event in runtime.poll_events(8, Duration::from_millis(20)) {
            if event.event_type == EventType::SessionOpened && event.endpoint == server {
                reopened_session = Some(event.session);
            }
            if event.event_type == EventType::Message && event.data == b"second" {
                assert!(reopened_session.is_some(), "message preceded SessionOpened");
                message_session = Some(event.session);
            }
        }
    }
    let reopened_session = reopened_session.expect("UDP session was not reopened");
    assert_ne!(reopened_session, first.session);
    assert_eq!(message_session, Some(reopened_session));
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn oversized_message_is_rejected_before_enqueue() {
    let runtime = test_runtime();
    let endpoint = runtime
        .open_endpoint(EndpointConfig::udp(
            "127.0.0.1:0".parse().unwrap(),
            Some("127.0.0.1:9".parse().unwrap()),
        ))
        .unwrap();
    let session = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == endpoint
    })
    .session;

    let error = runtime.send(session, 1, &[0; 1025]).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MessageTooLarge);
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn send_rejects_a_frame_that_exceeds_runtime_or_session_byte_budget() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        worker_threads: 2,
        max_runtime_queued_bytes: 28,
        max_session_queued_bytes: 28,
        ..RuntimeConfig::default()
    })
    .unwrap();
    let listener = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let client = runtime
        .open_endpoint(EndpointConfig::tcp_client(
            runtime.endpoint_local_addr(listener).unwrap(),
        ))
        .unwrap();
    let client_session = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == client
    })
    .session;

    let error = runtime.send(client_session, 1, b"x").unwrap_err();
    assert_eq!(error.code(), ErrorCode::WouldBlock);
    assert_eq!(runtime.metrics_snapshot().queued_send_bytes, 0);
}

#[test]
fn prometheus_snapshot_exports_resource_and_p999_metrics() {
    let runtime = NetworkRuntime::new(RuntimeConfig::default()).unwrap();

    let text = runtime.prometheus_snapshot();

    assert!(text.contains("rnet_queued_send_bytes 0"));
    assert!(text.contains("quantile=\"0.999\""));
    assert!(text.contains("rnet_admission_rejected_total"));
}

#[test]
fn legacy_plaintext_kcp_endpoint_is_explicitly_not_supported() {
    let runtime = test_runtime();
    let error = runtime
        .open_endpoint(EndpointConfig::kcp("127.0.0.1:0".parse().unwrap()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::NotSupported);
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn endpoint_open_reports_backpressure_when_its_event_cannot_be_queued() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        event_queue_capacity: 1,
        ..RuntimeConfig::default()
    })
    .unwrap();

    let error = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::WouldBlock);

    let started = runtime.poll_events(1, Duration::ZERO);
    assert_eq!(started[0].event_type, EventType::RuntimeStarted);
    let endpoint = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap();
    assert_ne!(endpoint, 0);
    let opened = runtime.poll_events(1, Duration::ZERO);
    assert_eq!(opened[0].event_type, EventType::EndpointOpened);
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn udp_never_delivers_a_new_peer_message_before_session_open() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        event_queue_capacity: 1,
        ..RuntimeConfig::default()
    })
    .unwrap();
    runtime.poll_events(1, Duration::ZERO);

    let server = runtime
        .open_endpoint(EndpointConfig::udp("127.0.0.1:0".parse().unwrap(), None))
        .unwrap();
    runtime.poll_events(1, Duration::ZERO);
    let server_address = runtime.endpoint_local_addr(server).unwrap();
    let client = runtime
        .open_endpoint(EndpointConfig::udp(
            "127.0.0.1:0".parse().unwrap(),
            Some(server_address),
        ))
        .unwrap();

    let opened = runtime.poll_events(1, Duration::from_secs(1));
    assert_eq!(opened[0].event_type, EventType::EndpointOpened);
    assert_eq!(opened[0].endpoint, client);
    let client_session = runtime.poll_events(1, Duration::from_secs(1))[0].session;

    runtime.send(client_session, 1, b"first").unwrap();
    let server_open = runtime.poll_events(1, Duration::from_secs(1));
    assert_eq!(server_open[0].event_type, EventType::SessionOpened);
    assert_eq!(server_open[0].endpoint, server);

    runtime.send(client_session, 1, b"second").unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let second = loop {
        assert!(
            Instant::now() < deadline,
            "second UDP message was not delivered"
        );
        let events = runtime.poll_events(1, Duration::from_millis(20));
        if let Some(message) = events.first() {
            assert_eq!(message.event_type, EventType::Message);
            assert_eq!(message.session, server_open[0].session);
            if message.data == b"second" {
                break message.clone();
            }
            assert_eq!(message.data, b"first");
        }
    };
    assert_eq!(second.data, b"second");
    loop {
        match runtime.stop(Duration::ZERO) {
            Ok(()) => break,
            Err(error) => {
                assert_eq!(error.code(), ErrorCode::WouldBlock);
                runtime.poll_events(1, Duration::ZERO);
            }
        }
    }
}

#[test]
fn stopped_event_does_not_replace_an_older_lifecycle_event() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        event_queue_capacity: 1,
        ..RuntimeConfig::default()
    })
    .unwrap();
    runtime.poll_events(1, Duration::ZERO);
    let endpoint = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap();

    runtime.stop(Duration::ZERO).unwrap();
    let retained = runtime.poll_events(1, Duration::ZERO);
    assert_eq!(retained[0].event_type, EventType::EndpointOpened);
    assert_eq!(runtime.metrics_snapshot().lifecycle_events_rejected, 1);
    assert_ne!(endpoint, 0);
    runtime.stop(Duration::ZERO).unwrap();
    assert!(runtime.poll_events(1, Duration::ZERO).is_empty());
}

#[test]
fn ipv6_tcp_and_udp_loopback_are_supported() {
    let tcp = test_runtime();
    let listener = tcp
        .open_endpoint(EndpointConfig::tcp_listener("[::1]:0".parse().unwrap()))
        .unwrap();
    let client = tcp
        .open_endpoint(EndpointConfig::tcp_client(
            tcp.endpoint_local_addr(listener).unwrap(),
        ))
        .unwrap();
    let (client_open, server_open) = poll_session_pair(&tcp, client, listener);
    tcp.send(client_open.session, 1, b"ipv6-tcp").unwrap();
    let message = poll_until(&tcp, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.session == server_open.session
    });
    assert_eq!(message.data, b"ipv6-tcp");
    tcp.stop(Duration::ZERO).unwrap();

    let udp = test_runtime();
    let server = udp
        .open_endpoint(EndpointConfig::udp("[::1]:0".parse().unwrap(), None))
        .unwrap();
    let client = udp
        .open_endpoint(EndpointConfig::udp(
            "[::1]:0".parse().unwrap(),
            Some(udp.endpoint_local_addr(server).unwrap()),
        ))
        .unwrap();
    let client_session = poll_until(&udp, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == client
    })
    .session;
    udp.send(client_session, 2, b"ipv6-udp").unwrap();
    let message = poll_until(&udp, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message && event.endpoint == server
    });
    assert_eq!(message.data, b"ipv6-udp");
    udp.stop(Duration::ZERO).unwrap();
}

#[test]
fn tcp_reports_port_conflict_and_remote_half_close() {
    let runtime = test_runtime();
    let listener = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = runtime.endpoint_local_addr(listener).unwrap();
    let conflict = runtime
        .open_endpoint(EndpointConfig::tcp_listener(address))
        .unwrap_err();
    assert_eq!(conflict.code(), ErrorCode::IoError);

    let stream = StdTcpStream::connect(address).unwrap();
    let opened = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == listener
    });
    stream.shutdown(Shutdown::Write).unwrap();
    let closed = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionClosed && event.session == opened.session
    });
    assert_eq!(closed.endpoint, listener);
    assert_eq!(closed.status, ErrorCode::IoError);
    assert_eq!(
        runtime
            .metrics_snapshot()
            .closed_sessions(ErrorCode::IoError),
        1
    );
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn compatibility_tcp_listener_enforces_session_admission_limit() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        max_sessions_per_endpoint: 1,
        ..RuntimeConfig::default()
    })
    .unwrap();
    let listener = runtime
        .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = runtime.endpoint_local_addr(listener).unwrap();
    let _first = StdTcpStream::connect(address).unwrap();
    let _opened = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == listener
    });
    let _second = StdTcpStream::connect(address).unwrap();

    let unexpected = runtime
        .poll_events(16, Duration::from_millis(100))
        .into_iter()
        .any(|event| event.event_type == EventType::SessionOpened && event.endpoint == listener);
    assert!(
        !unexpected,
        "listener admitted more sessions than configured"
    );
}
