use super::*;
use crate::envelope::{encode_control, ControlKind};
use rnet_core::Transport;
use rnet_security::Keypair;

#[test]
fn heartbeat_ticks_do_not_shorten_the_requested_poll_timeout() {
    let runtime = GameRuntime::new(
        GameRuntimeConfig::production()
            .with_heartbeat(Duration::from_secs(1), Duration::from_secs(3)),
    )
    .expect("runtime");
    runtime.poll(1, Duration::ZERO); // Consume RuntimeStarted before measuring an empty poll.
    runtime.track_session(u64::MAX);
    let started = Instant::now();
    assert!(runtime.poll(1, Duration::from_millis(130)).is_empty());
    assert!(started.elapsed() >= Duration::from_millis(110));
}

#[test]
fn queued_public_events_do_not_starve_expired_heartbeat_checks() {
    let runtime = GameRuntime::new(
        GameRuntimeConfig::production()
            .with_heartbeat(Duration::from_millis(10), Duration::from_millis(100)),
    )
    .expect("runtime");
    let old = Instant::now() - Duration::from_secs(1);
    let mut tracker =
        HeartbeatTracker::new(Duration::from_millis(10), Duration::from_millis(100), old)
            .expect("tracker");
    tracker
        .mark_sent(7, old + Duration::from_millis(10))
        .expect("outstanding probe");
    runtime
        .heartbeat_trackers
        .lock()
        .expect("trackers")
        .insert(42, tracker);

    assert!(matches!(
        runtime.poll(1, Duration::ZERO).as_slice(),
        [GameEvent::RuntimeStarted]
    ));
    assert_eq!(runtime.heartbeat_metrics_snapshot().timeouts, 1);
}

#[test]
fn zero_capacity_poll_still_advances_heartbeat_timeouts() {
    let runtime = GameRuntime::new(
        GameRuntimeConfig::production()
            .with_heartbeat(Duration::from_millis(10), Duration::from_millis(100)),
    )
    .expect("runtime");
    let old = Instant::now() - Duration::from_secs(1);
    let mut tracker =
        HeartbeatTracker::new(Duration::from_millis(10), Duration::from_millis(100), old)
            .expect("tracker");
    tracker
        .mark_sent(7, old + Duration::from_millis(10))
        .expect("outstanding probe");
    runtime
        .heartbeat_trackers
        .lock()
        .expect("trackers")
        .insert(42, tracker);

    assert!(runtime.poll(0, Duration::from_secs(1)).is_empty());
    assert_eq!(runtime.heartbeat_metrics_snapshot().timeouts, 1);
}

#[test]
fn zero_capacity_poll_stages_public_events_in_the_bounded_completed_queue() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    for protocol_id in 100..104 {
        runtime
            .listen(GameServerConfig {
                transport: Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().expect("address"),
                local_key: Keypair::generate().expect("key"),
                initial_encryption: true,
                protocol: GameProtocol::new(protocol_id, 1),
            })
            .expect("listener");
    }

    for _ in 0..32 {
        assert!(runtime.poll(0, Duration::ZERO).is_empty());
    }
    let completed = runtime
        .range
        .lock()
        .expect("range")
        .completed_len_for_test();
    assert!(
        (1..=5).contains(&completed),
        "zero-capacity maintenance must retain public events in a bounded queue"
    );
}

#[test]
fn completed_backlog_does_not_starve_authenticated_heartbeat_controls() {
    let key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production()
            .with_heartbeat(Duration::from_millis(20), Duration::from_millis(200)),
        ClientSecurity::pinned(client_key, key.public.clone()),
    )
    .expect("runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            local_key: key,
            initial_encryption: true,
            protocol: GameProtocol::new(91, 1),
        })
        .expect("listener");
    let client = runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"join".to_vec(),
            protocol: GameProtocol::new(91, 1),
        })
        .expect("client");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut ready = 0;
    while ready < 2 {
        assert!(Instant::now() < deadline, "sessions did not become ready");
        for event in runtime.poll(16, Duration::from_millis(5)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).expect("authorize")
                }
                GameEvent::SessionReady { .. } => ready += 1,
                _ => {}
            }
        }
    }
    assert_ne!(listener, client);
    for index in 0..100 {
        runtime
            .range
            .lock()
            .expect("range")
            .push_completed_for_test(GameEvent::Writable {
                endpoint: 999,
                session: 10_000 + index,
            });
    }
    let deadline = Instant::now() + Duration::from_millis(350);
    while Instant::now() < deadline {
        assert_eq!(runtime.poll(1, Duration::ZERO).len(), 1);
        std::thread::sleep(Duration::from_millis(5));
    }
    let metrics = runtime.heartbeat_metrics_snapshot();
    assert_eq!(
        metrics.timeouts, 0,
        "completed events starved heartbeat ACKs"
    );
    assert!(
        metrics.replies_matched >= 1,
        "heartbeat ACKs were not consumed"
    );
}

#[test]
fn zero_capacity_poll_with_completed_backlog_still_consumes_heartbeat_controls() {
    let key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production()
            .with_heartbeat(Duration::from_millis(20), Duration::from_millis(200)),
        ClientSecurity::pinned(client_key, key.public.clone()),
    )
    .expect("runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            local_key: key,
            initial_encryption: true,
            protocol: GameProtocol::new(92, 1),
        })
        .expect("listener");
    let _client = runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"join".to_vec(),
            protocol: GameProtocol::new(92, 1),
        })
        .expect("client");
    let ready_deadline = Instant::now() + Duration::from_secs(2);
    let mut ready = 0;
    while ready < 2 {
        assert!(
            Instant::now() < ready_deadline,
            "sessions did not become ready"
        );
        for event in runtime.poll(16, Duration::from_millis(5)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).expect("authorize")
                }
                GameEvent::SessionReady { .. } => ready += 1,
                _ => {}
            }
        }
    }
    runtime
        .range
        .lock()
        .expect("range")
        .push_completed_for_test(GameEvent::Writable {
            endpoint: 999,
            session: 10_000,
        });

    let heartbeat_deadline = Instant::now() + Duration::from_millis(350);
    while Instant::now() < heartbeat_deadline {
        assert!(runtime.poll(0, Duration::ZERO).is_empty());
        std::thread::sleep(Duration::from_millis(5));
    }

    let metrics = runtime.heartbeat_metrics_snapshot();
    assert_eq!(metrics.timeouts, 0, "queued heartbeat ACKs were starved");
    assert!(
        metrics.replies_matched >= 1,
        "heartbeat ACKs were not consumed"
    );
    assert!(
        runtime
            .poll(16, Duration::ZERO)
            .iter()
            .any(|event| matches!(
                event,
                GameEvent::Writable {
                    session: 10_000,
                    ..
                }
            )),
        "zero-capacity maintenance must preserve the original public event"
    );
}

#[test]
fn runtime_rejects_invalid_heartbeat_policy_before_starting_threads() {
    let error = GameRuntime::new(
        GameRuntimeConfig::production().with_heartbeat(Duration::ZERO, Duration::from_secs(1)),
    )
    .err()
    .expect("zero heartbeat interval must be rejected");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn runtime_rejects_unordered_quality_thresholds() {
    let policy = crate::quality::QualityPolicy {
        good_udp_loss_per_mille: 1,
        ..Default::default()
    };
    let error = GameRuntime::new(GameRuntimeConfig::production().with_quality_policy(policy))
        .err()
        .expect("unordered quality policy must be rejected");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn runtime_rejects_invalid_realtime_queue_limits() {
    let queue = crate::realtime::RealtimeQueueConfig {
        flush_batch: 0,
        ..Default::default()
    };
    let error = GameRuntime::new(GameRuntimeConfig::production().with_realtime_queue(queue))
        .err()
        .expect("zero flush batch must be rejected");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn range_early_data_budget_is_visible_without_session_labels() {
    let mut config = GameRuntimeConfig::production();
    config.network.event_queue_capacity = 17;
    config.network.max_event_bytes = 4_321;
    let runtime = GameRuntime::new(config).expect("runtime");

    let snapshot = runtime.range_buffer_snapshot();
    assert_eq!(snapshot.max_buffered_messages, 17);
    assert_eq!(snapshot.max_buffered_bytes, 4_321);
    assert_eq!(snapshot.buffered_messages, 0);
    assert_eq!(snapshot.buffered_bytes, 0);
    let prometheus = runtime.prometheus_snapshot();
    assert!(prometheus.contains("rnet_game_range_early_max_buffered_messages 17"));
    assert!(prometheus.contains("rnet_game_range_early_max_buffered_bytes 4321"));
    assert!(!prometheus.contains("session="));
}

#[test]
fn udp_sequence_extension_is_required_only_on_udp_game_sessions() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    runtime.track_session(41);
    runtime.track_quality_session(41, Transport::Udp);
    assert_eq!(
        runtime
            .observe_datagram_sequence(41, None)
            .expect_err("UDP must carry the v2 sequence")
            .code(),
        ErrorCode::ProtocolError
    );
    runtime
        .observe_datagram_sequence(41, Some(0))
        .expect("first UDP sequence");
    assert_eq!(
        runtime
            .udp_loss_snapshot(41)
            .expect("UDP snapshot")
            .unwrap()
            .received,
        1
    );

    runtime.track_session(42);
    assert_eq!(
        runtime
            .observe_datagram_sequence(42, Some(0))
            .expect_err("TCP/KCP must reject a UDP-only extension")
            .code(),
        ErrorCode::ProtocolError
    );
    assert_eq!(
        runtime.udp_loss_snapshot(42).expect("non-UDP snapshot"),
        None
    );
}

#[test]
fn plaintext_game_controls_cannot_impersonate_authenticated_controls() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    let mut event = Event::simple(EventType::Message);
    event.session = 1;
    event.data = encode_control(ControlKind::Heartbeat, b"", 1024)
        .expect("control")
        .to_vec();

    let error = runtime
        .convert_event(event)
        .expect_err("unhandled control must fail closed");
    assert_eq!(error.code(), ErrorCode::ProtocolError);
}

#[test]
fn malformed_plaintext_business_frame_does_not_close_a_ready_session() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production().allow_plaintext_business_data(true),
        client_security,
    )
    .expect("runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            local_key: server_key,
            initial_encryption: false,
            protocol: GameProtocol::new(93, 1),
        })
        .expect("listener");
    let client_endpoint = runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"ticket".to_vec(),
            protocol: GameProtocol::new(93, 1),
        })
        .expect("connect");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut server_session = None;
    let mut client_session = None;
    while server_session.is_none() || client_session.is_none() {
        assert!(Instant::now() < deadline, "game session did not open");
        for event in runtime.poll(16, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).expect("authorize");
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                    server_session = Some(session);
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == client_endpoint => {
                    client_session = Some(session);
                }
                _ => {}
            }
        }
    }
    let server_session = server_session.expect("server session");
    let client_session = client_session.expect("client session");
    let forged = encode_control(ControlKind::Heartbeat, b"", 1024).expect("control");
    runtime
        .network
        .send(client_session, 0, &forged)
        .expect("send forged business frame");
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        assert!(
            !runtime
                .poll(16, Duration::from_millis(10))
                .iter()
                .any(|event| matches!(event, GameEvent::ProtocolViolation { session, .. } if *session == server_session)),
            "unauthenticated plaintext corruption must not reach the game loop"
        );
    }
    assert!(runtime
        .prometheus_snapshot()
        .contains("rnet_game_plaintext_invalid_envelopes_dropped_total 1"));
    runtime
        .send(client_session, b"still-alive")
        .expect("client remains connected");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            Instant::now() < deadline,
            "valid business message did not arrive"
        );
        if runtime
            .poll(16, Duration::from_millis(10))
            .iter()
            .any(|event| matches!(event, GameEvent::Message(message) if message.session == server_session && message.payload.as_ref() == b"still-alive"))
        {
            break;
        }
    }
    let unsupported = encode_control(ControlKind::ClockSync, b"", 1024)
        .expect("unsupported authenticated control");
    runtime
        .network
        .send_game_control(client_session, &unsupported)
        .expect("send authenticated control");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            Instant::now() < deadline,
            "authenticated violation not reported"
        );
        if runtime
            .poll(16, Duration::from_millis(10))
            .iter()
            .any(|event| matches!(event, GameEvent::ProtocolViolation { session, .. } if *session == server_session))
        {
            break;
        }
    }
    assert_eq!(
        runtime
            .network
            .validate_payload_len(server_session, 0)
            .expect_err("authenticated control violation closes session")
            .code(),
        ErrorCode::InvalidHandle
    );
}

#[test]
fn encrypted_business_violation_closes_even_when_plaintext_is_permitted() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        assert_encrypted_business_violation_closes(transport);
    }
}

fn assert_encrypted_business_violation_closes(transport: Transport) {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production().allow_plaintext_business_data(true),
        client_security,
    )
    .expect("runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport,
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            local_key: server_key,
            initial_encryption: false,
            protocol: GameProtocol::new(94, 1),
        })
        .expect("listener");
    let client_endpoint = runtime
        .connect(GameClientConfig {
            transport,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"ticket".to_vec(),
            protocol: GameProtocol::new(94, 1),
        })
        .expect("connect");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut server_session = None;
    let mut client_session = None;
    while server_session.is_none() || client_session.is_none() {
        assert!(Instant::now() < deadline, "game session did not open");
        for event in runtime.poll(16, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).expect("authorize");
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                    server_session = Some(session);
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == client_endpoint => {
                    client_session = Some(session);
                }
                _ => {}
            }
        }
    }
    let server_session = server_session.expect("server session");
    let client_session = client_session.expect("client session");
    runtime
        .set_encryption(server_session, true)
        .expect("enable encryption");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut changed = 0;
    while changed < 2 {
        assert!(
            Instant::now() < deadline,
            "security transition did not complete"
        );
        changed += runtime
            .poll(16, Duration::from_millis(10))
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    GameEvent::SecurityChanged {
                        encrypted: true,
                        ..
                    }
                )
            })
            .count();
    }
    let forged = encode_control(ControlKind::Heartbeat, b"", 1024).expect("control");
    runtime
        .network
        .send(client_session, 0, &forged)
        .expect("send authenticated business frame");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(Instant::now() < deadline, "violation not reported");
        if runtime
            .poll(16, Duration::from_millis(10))
            .iter()
            .any(|event| matches!(event, GameEvent::ProtocolViolation { session, .. } if *session == server_session))
        {
            break;
        }
    }
    assert_eq!(
        runtime
            .network
            .validate_payload_len(server_session, 0)
            .expect_err("authenticated violation closes session")
            .code(),
        ErrorCode::InvalidHandle
    );
}

#[test]
fn queued_authenticated_control_after_local_close_is_not_a_protocol_violation() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    let mut event = Event::simple(EventType::GameControl);
    event.session = 42;
    event.data = crate::heartbeat::encode_heartbeat(
        crate::heartbeat::HeartbeatPacket {
            kind: crate::heartbeat::HeartbeatKind::Probe,
            challenge: 7,
        },
        1024,
    )
    .expect("heartbeat")
    .to_vec();
    assert_eq!(
        runtime
            .convert_event(event)
            .expect("stale control is ignored"),
        None
    );
    assert_eq!(
        runtime.heartbeat_metrics_snapshot().probes_rate_limited,
        0,
        "closed sessions must not be mistaken for rate-limited live sessions"
    );
}

#[test]
fn unmatched_heartbeat_ack_does_not_enter_latency_distribution() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    runtime.track_session(42);
    let mut event = Event::simple(EventType::GameControl);
    event.session = 42;
    event.data = crate::heartbeat::encode_heartbeat(
        crate::heartbeat::HeartbeatPacket {
            kind: crate::heartbeat::HeartbeatKind::Ack,
            challenge: 7,
        },
        1024,
    )
    .expect("heartbeat")
    .to_vec();

    assert_eq!(runtime.convert_event(event).expect("ack ignored"), None);
    let metrics = runtime.heartbeat_metrics_snapshot();
    assert_eq!(metrics.replies_rejected, 1);
    assert_eq!(metrics.replies_matched, 0);
    assert_eq!(metrics.rtt.sample_count, 0);
    assert_eq!(runtime.drain_heartbeat_rtt_window().sample_count, 0);
}

#[test]
fn transient_endpoint_error_does_not_erase_listener_protocol_gate() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    let endpoint = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().expect("address"),
            local_key: Keypair::generate().expect("key"),
            initial_encryption: true,
            protocol: GameProtocol::new(12, 1),
        })
        .expect("listener");
    let mut error = Event::simple(EventType::EndpointError);
    error.endpoint = endpoint;
    error.status = ErrorCode::IoError;

    assert!(matches!(
        runtime.convert_event(error),
        Ok(Some(GameEvent::EndpointError { .. }))
    ));
    assert_eq!(
        runtime
            .server_protocols
            .lock()
            .expect("protocol table")
            .get(&endpoint),
        Some(&GameProtocol::new(12, 1))
    );

    runtime.close_endpoint(endpoint).expect("close listener");
    assert!(!runtime
        .server_protocols
        .lock()
        .expect("protocol table")
        .contains_key(&endpoint));
}

#[test]
fn runtime_rejects_body_limits_that_cannot_carry_clock_reply() {
    let mut config = GameRuntimeConfig::production();
    config.network.max_body_len = 44;

    let error = GameRuntime::new(config)
        .err()
        .expect("impossible protected clock reply must fail at construction");
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
}

#[test]
fn stopping_runtime_closes_all_game_send_admission() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");
    runtime.stop(Duration::ZERO).expect("stop runtime");

    assert_eq!(
        runtime.send(1, b"late").expect_err("scheduled send").code(),
        ErrorCode::InvalidState
    );
    assert_eq!(
        runtime
            .send_latest(1, 9, b"late")
            .expect_err("latest send")
            .code(),
        ErrorCode::InvalidState
    );
}

#[test]
fn invalid_stop_deadline_does_not_close_game_send_admission() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).expect("runtime");

    assert_eq!(
        runtime
            .stop(Duration::MAX)
            .expect_err("unrepresentable deadline")
            .code(),
        ErrorCode::InvalidArgument
    );
    assert!(!runtime.admission.is_stopping());

    runtime.stop(Duration::ZERO).expect("valid stop");
}
