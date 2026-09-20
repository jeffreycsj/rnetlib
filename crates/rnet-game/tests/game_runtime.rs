use rnet_core::Transport;
use rnet_game::{
    GameClientConfig, GameEvent, GameHostClientConfig, GameProtocol, GameRuntime,
    GameRuntimeConfig, GameSendOptions, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::{ClientSecurity, SecurityOperation};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

fn localhost(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

#[test]
fn game_api_sends_opaque_payload_and_follows_server_encryption_changes() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        exercise_game_session(transport);
    }
}

#[test]
fn heartbeat_quality_uses_protected_controls_even_when_business_data_is_plaintext() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let server_key = Keypair::generate().expect("server key");
        let client_key = Keypair::generate().expect("client key");
        let security = ClientSecurity::pinned(client_key, server_key.public.clone());
        let runtime = GameRuntime::new_with_client_security(
            GameRuntimeConfig::production()
                .allow_plaintext_business_data(true)
                .with_heartbeat(Duration::from_millis(20), Duration::from_millis(500)),
            security,
        )
        .expect("game runtime");
        let listener = runtime
            .listen(GameServerConfig {
                transport,
                bind_addr: localhost(0),
                local_key: server_key,
                initial_encryption: false,
                protocol: GameProtocol::new(10, 1),
            })
            .expect("listener");
        let client = runtime
            .connect(GameClientConfig {
                transport,
                bind_addr: None,
                remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
                join_ticket: b"ticket".to_vec(),
                protocol: GameProtocol::new(10, 1),
            })
            .expect("client");
        let auth = poll_until(&runtime, |event| {
            matches!(event, GameEvent::AuthRequest { .. })
        });
        let GameEvent::AuthRequest { session, .. } = auth else {
            unreachable!();
        };
        runtime.auth_decide(session, true).expect("authorize");
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut server_session = None;
        let mut client_session = None;
        while Instant::now() < deadline && (server_session.is_none() || client_session.is_none()) {
            for event in runtime.poll(32, Duration::from_millis(10)) {
                if let GameEvent::SessionReady { endpoint, session } = event {
                    if endpoint == listener {
                        server_session = Some(session);
                    } else if endpoint == client {
                        client_session = Some(session);
                    }
                }
            }
        }
        let server_session = server_session.expect("server ready");
        let client_session = client_session.expect("client ready");
        let deadline = Instant::now() + Duration::from_secs(3);
        let quality = loop {
            let events = runtime.poll(32, Duration::from_millis(10));
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, GameEvent::Message(_))),
                "internal heartbeat leaked as a business message"
            );
            if let Some(quality) = runtime
                .network_quality(server_session)
                .expect("server quality")
            {
                break quality;
            }
            assert!(Instant::now() < deadline, "heartbeat sample timed out");
        };
        assert!(quality.samples >= 1);
        assert!(quality.last_rtt < Duration::from_millis(500));
        assert!(runtime.network_quality(client_session).is_ok());
        runtime
            .set_encryption(server_session, true)
            .expect("enable business encryption while heartbeats continue");
        await_security_change(
            &runtime,
            server_session,
            client_session,
            SecurityOperation::ModeSwitch,
            true,
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while runtime
            .network_quality(server_session)
            .expect("quality after mode change")
            .is_none_or(|sample| sample.samples <= quality.samples)
        {
            runtime.poll(32, Duration::from_millis(10));
            assert!(
                Instant::now() < deadline,
                "heartbeats stalled after mode change"
            );
        }
        runtime.stop(Duration::ZERO).expect("stop runtime");
    }
}

#[test]
fn absent_heartbeat_ack_closes_only_the_unresponsive_session() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let policy = GameRuntimeConfig::production()
        .with_heartbeat(Duration::from_millis(20), Duration::from_millis(100));
    let server = GameRuntime::new(policy.clone()).expect("server runtime");
    let client = GameRuntime::new_with_client_security(policy, security).expect("client runtime");
    let listener = server
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(11, 1),
        })
        .expect("listener");
    client
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: server.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"ticket".to_vec(),
            protocol: GameProtocol::new(11, 1),
        })
        .expect("client connect");
    let auth = poll_until(&server, |event| {
        matches!(event, GameEvent::AuthRequest { .. })
    });
    let GameEvent::AuthRequest { session, .. } = auth else {
        unreachable!();
    };
    server.auth_decide(session, true).expect("authorize");
    poll_until(
        &server,
        |event| matches!(event, GameEvent::SessionReady { session: ready, .. } if *ready == session),
    );
    // The client transport remains connected, but its game loop deliberately never polls the
    // authenticated probe or sends an acknowledgement.
    let closed = poll_until(
        &server,
        |event| matches!(event, GameEvent::SessionClosed { session: closed, reason: rnet_core::ErrorCode::Timeout, .. } if *closed == session),
    );
    assert!(matches!(closed, GameEvent::SessionClosed { .. }));
    assert_eq!(
        server
            .network_quality(session)
            .expect_err("closed session has no quality")
            .code(),
        rnet_core::ErrorCode::InvalidHandle
    );
    server.stop(Duration::ZERO).expect("stop server");
    client.stop(Duration::ZERO).expect("stop client");
}

#[test]
fn timely_ack_queued_before_a_late_poll_prevents_false_timeout() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let policy = GameRuntimeConfig::production()
        .with_heartbeat(Duration::from_millis(20), Duration::from_millis(200));
    let server = GameRuntime::new(policy.clone()).expect("server runtime");
    let client = GameRuntime::new_with_client_security(policy, security).expect("client runtime");
    let listener = server
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(12, 1),
        })
        .expect("listener");
    let client_endpoint = client
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: server.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"ticket".to_vec(),
            protocol: GameProtocol::new(12, 1),
        })
        .expect("client connect");
    let auth = poll_until(&server, |event| {
        matches!(event, GameEvent::AuthRequest { .. })
    });
    let GameEvent::AuthRequest { session, .. } = auth else {
        unreachable!();
    };
    server.auth_decide(session, true).expect("authorize");
    poll_until(
        &server,
        |event| matches!(event, GameEvent::SessionReady { session: ready, .. } if *ready == session),
    );
    poll_until(
        &client,
        |event| matches!(event, GameEvent::SessionReady { endpoint, .. } if *endpoint == client_endpoint),
    );
    std::thread::sleep(Duration::from_millis(25));
    server.poll(32, Duration::ZERO); // Enqueue one authenticated probe.
    let client_deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < client_deadline {
        client.poll(32, Duration::from_millis(5)); // Process probe and enqueue ack.
    }
    std::thread::sleep(Duration::from_millis(220));
    let events = server.poll(32, Duration::ZERO);
    assert!(
        !events.iter().any(|event| matches!(event, GameEvent::SessionClosed { session: closed, reason: rnet_core::ErrorCode::Timeout, .. } if *closed == session)),
        "an on-time queued ack must be processed before timeout"
    );
    assert!(
        server
            .network_quality(session)
            .expect("session remains ready")
            .is_some(),
        "the queued ack must update quality"
    );
    server.stop(Duration::ZERO).expect("stop server");
    client.stop(Duration::ZERO).expect("stop client");
}

fn exercise_game_session(transport: Transport) {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production().allow_plaintext_business_data(true),
        client_security,
    )
    .expect("game runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_encryption: false,
            protocol: GameProtocol::new(77, 3),
        })
        .expect("listener");
    let client = runtime
        .connect(GameClientConfig {
            transport,
            bind_addr: None,
            remote_addr: runtime
                .endpoint_local_addr(listener)
                .expect("listener address"),
            join_ticket: b"player-ticket".to_vec(),
            protocol: GameProtocol::new(77, 3),
        })
        .expect("client");

    let auth = poll_until(&runtime, |event| {
        matches!(event, GameEvent::AuthRequest { .. })
    });
    let (auth_session, join_ticket) = match auth {
        GameEvent::AuthRequest {
            session,
            join_ticket,
            ..
        } => (session, join_ticket),
        _ => unreachable!(),
    };
    assert_eq!(join_ticket, b"player-ticket");
    runtime.auth_decide(auth_session, true).expect("authorize");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut server_session = None;
    let mut client_session = None;
    while Instant::now() < deadline && (server_session.is_none() || client_session.is_none()) {
        for event in runtime.poll(32, Duration::from_millis(20)) {
            if let GameEvent::SessionReady { endpoint, session } = event {
                if endpoint == listener {
                    server_session = Some(session);
                } else if endpoint == client {
                    client_session = Some(session);
                }
            }
        }
    }
    let server_session = server_session.expect("server session ready");
    let client_session = client_session.expect("client session ready");

    // There is deliberately no message type or transport argument here.
    runtime
        .send(client_session, b"protobuf-payload")
        .expect("send plaintext payload");
    assert_eq!(
        assert_message(&runtime, server_session, b"protobuf-payload").tick,
        None
    );

    assert_eq!(
        runtime
            .set_encryption(client_session, true)
            .expect_err("a client cannot control encryption")
            .code(),
        rnet_core::ErrorCode::InvalidState
    );
    assert_eq!(
        runtime
            .rekey(client_session)
            .expect_err("a client cannot rotate session keys")
            .code(),
        rnet_core::ErrorCode::InvalidState
    );

    runtime
        .set_encryption(server_session, true)
        .expect("server enables encryption");
    let encrypted_epoch = await_security_change(
        &runtime,
        server_session,
        client_session,
        SecurityOperation::ModeSwitch,
        true,
    );
    runtime
        .send(client_session, b"same-api-encrypted")
        .expect("send encrypted payload");
    assert_message(&runtime, server_session, b"same-api-encrypted");

    runtime
        .rekey(server_session)
        .expect("server rekeys session");
    let rekey_epoch = await_security_change(
        &runtime,
        server_session,
        client_session,
        SecurityOperation::Rekey,
        true,
    );
    assert!(rekey_epoch > encrypted_epoch);
    runtime
        .send(client_session, b"same-api-rekeyed")
        .expect("send after rekey");
    assert_message(&runtime, server_session, b"same-api-rekeyed");

    runtime
        .set_encryption(server_session, false)
        .expect("server disables encryption");
    let plaintext_epoch = await_security_change(
        &runtime,
        server_session,
        client_session,
        SecurityOperation::ModeSwitch,
        false,
    );
    assert!(plaintext_epoch > rekey_epoch);
    runtime
        .send(client_session, b"same-api-plaintext-again")
        .expect("send plaintext again");
    assert_message(&runtime, server_session, b"same-api-plaintext-again");

    runtime
        .send_with_options(
            client_session,
            b"tick-owned-by-envelope",
            GameSendOptions {
                sequence: Some(u32::MAX),
                tick: Some(60),
            },
        )
        .expect("send with network metadata");
    let message = assert_message(&runtime, server_session, b"tick-owned-by-envelope");
    assert_eq!(message.sequence, Some(u32::MAX));
    assert_eq!(message.tick, Some(60));

    let metrics = runtime.metrics_snapshot();
    assert!(metrics.frames_sent >= 5);
    assert!(runtime
        .prometheus_snapshot()
        .contains("rnet_frames_sent_total"));
    runtime
        .close_session(client_session)
        .expect("close game session");
    runtime.stop(Duration::ZERO).expect("stop game runtime");
}

#[test]
fn incompatible_protocol_is_rejected_before_business_authorization() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime =
        GameRuntime::new_with_client_security(GameRuntimeConfig::production(), client_security)
            .expect("game runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(77, 3),
        })
        .expect("listener");
    runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).expect("address"),
            join_ticket: b"secret-ticket".to_vec(),
            protocol: GameProtocol::new(77, 4),
        })
        .expect("client");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut rejected = false;
    while Instant::now() < deadline && !rejected {
        for event in runtime.poll(32, Duration::from_millis(20)) {
            match event {
                GameEvent::ProtocolRejected { endpoint, .. } if endpoint == listener => {
                    rejected = true;
                }
                GameEvent::AuthRequest { .. } | GameEvent::SessionReady { .. } => {
                    panic!("incompatible peer reached business authorization or ready state")
                }
                _ => {}
            }
        }
    }
    assert!(rejected, "server did not report protocol rejection");
}

#[test]
fn client_can_join_by_hostname_without_exposing_transport_on_send() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let client_security = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime =
        GameRuntime::new_with_client_security(GameRuntimeConfig::production(), client_security)
            .expect("game runtime");
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: localhost(0),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(9, 1),
        })
        .expect("listener");
    let client = runtime
        .connect_host(GameHostClientConfig {
            transport: Transport::Tcp,
            host: "localhost".to_owned(),
            port: runtime
                .endpoint_local_addr(listener)
                .expect("address")
                .port(),
            protocol: GameProtocol::new(9, 1),
            join_ticket: b"ticket".to_vec(),
        })
        .expect("connect by name");
    let auth = poll_until(&runtime, |event| {
        matches!(event, GameEvent::AuthRequest { .. })
    });
    let GameEvent::AuthRequest { session, .. } = auth else {
        unreachable!();
    };
    runtime.auth_decide(session, true).expect("accept");
    let ready = poll_until(
        &runtime,
        |event| matches!(event, GameEvent::SessionReady { endpoint, .. } if *endpoint == client),
    );
    let GameEvent::SessionReady { session, .. } = ready else {
        unreachable!();
    };
    runtime.send(session, b"opaque").expect("send");
    let message = poll_until(&runtime, |event| matches!(event, GameEvent::Message(_)));
    let GameEvent::Message(message) = message else {
        unreachable!();
    };
    assert_eq!(message.payload, b"opaque".as_slice());
}

fn poll_until(runtime: &GameRuntime, predicate: impl Fn(&GameEvent) -> bool) -> GameEvent {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(event) = runtime
            .poll(32, Duration::from_millis(20))
            .into_iter()
            .find(&predicate)
        {
            return event;
        }
    }
    panic!("game event did not arrive before timeout");
}

fn await_security_change(
    runtime: &GameRuntime,
    server_session: u64,
    client_session: u64,
    operation: SecurityOperation,
    encrypted: bool,
) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut server_epoch = None;
    let mut client_epoch = None;
    while Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(20)) {
            if let GameEvent::SecurityChanged {
                session,
                encrypted: actual_mode,
                epoch,
                operation: actual_operation,
                ..
            } = event
            {
                if actual_operation == operation && actual_mode == encrypted {
                    if session == server_session {
                        server_epoch = Some(epoch);
                    } else if session == client_session {
                        client_epoch = Some(epoch);
                    }
                }
            }
        }
        if let (Some(server), Some(client)) = (server_epoch, client_epoch) {
            assert_eq!(server, client, "peers must commit the same security epoch");
            return server;
        }
    }
    panic!("both peers did not complete the security operation before timeout");
}

fn assert_message(runtime: &GameRuntime, session: u64, expected: &[u8]) -> rnet_game::GameMessage {
    let event = poll_until(
        runtime,
        |event| matches!(event, GameEvent::Message(message) if message.session == session),
    );
    let GameEvent::Message(message) = event else {
        unreachable!();
    };
    assert_eq!(message.payload.as_ref(), expected);
    message
}
