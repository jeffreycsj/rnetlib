use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    GameEvent, GameProtocolRange, GameRangeClientConfig, GameRangeServerConfig, GameRuntime,
    GameRuntimeConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::{Duration, Instant};

#[test]
fn v4_range_join_selects_server_highest_common_version_before_ready() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let server_key = Keypair::generate().unwrap();
        let client_key = Keypair::generate().unwrap();
        let runtime = GameRuntime::new_with_client_security(
            GameRuntimeConfig::production(),
            ClientSecurity::pinned(client_key, server_key.public.clone()),
        )
        .unwrap();
        let listener = runtime
            .listen_range(GameRangeServerConfig {
                transport,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: server_key,
                initial_encryption: true,
                protocol: GameProtocolRange::new(71, 3, 8),
            })
            .unwrap();
        let client = runtime
            .connect_range(GameRangeClientConfig {
                transport,
                bind_addr: None,
                remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
                join_ticket: b"join".to_vec(),
                protocol: GameProtocolRange::new(71, 5, 10),
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut server_ready = None;
        let mut client_ready = None;
        let mut auth_seen = false;
        while (server_ready.is_none() || client_ready.is_none()) && Instant::now() < deadline {
            for event in runtime.poll(32, Duration::from_millis(10)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        auth_seen = true;
                        assert_eq!(runtime.selected_protocol_version(session).unwrap(), 8);
                        assert_eq!(
                            runtime.send(session, b"too early").unwrap_err().code(),
                            ErrorCode::HandshakeRequired
                        );
                        runtime.auth_decide(session, true).unwrap();
                    }
                    GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                        server_ready = Some(session);
                    }
                    GameEvent::SessionReady { endpoint, session } if endpoint == client => {
                        client_ready = Some(session);
                    }
                    GameEvent::ProtocolViolation { .. } | GameEvent::ProtocolRejected { .. } => {
                        panic!("valid v4 join was rejected: {event:?}");
                    }
                    _ => {}
                }
            }
        }
        assert!(auth_seen, "server never received authorization request");
        let server_session = server_ready.expect("server v4 ready");
        let client_session = client_ready.expect("client v4 ready");
        assert_eq!(
            runtime.selected_protocol_version(server_session).unwrap(),
            8
        );
        assert_eq!(
            runtime.selected_protocol_version(client_session).unwrap(),
            8
        );
        runtime.send(client_session, b"v4").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut received = false;
        while !received && Instant::now() < deadline {
            received = runtime.poll(32, Duration::from_millis(10)).into_iter().any(|event| {
                matches!(event, GameEvent::Message(message) if message.session == server_session && message.payload == b"v4"[..])
            });
        }
        assert!(received, "v4 business payload was not delivered");
    }
}

#[test]
fn v4_range_mismatch_is_rejected_before_business_authorization() {
    let server_key = Keypair::generate().unwrap();
    let client_key = Keypair::generate().unwrap();
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production(),
        ClientSecurity::pinned(client_key, server_key.public.clone()),
    )
    .unwrap();
    let listener = runtime
        .listen_range(GameRangeServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocolRange::new(72, 1, 2),
        })
        .unwrap();
    runtime
        .connect_range(GameRangeClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_ticket: b"join".to_vec(),
            protocol: GameProtocolRange::new(72, 3, 4),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        for event in runtime.poll(16, Duration::from_millis(10)) {
            assert!(!matches!(event, GameEvent::AuthRequest { .. }));
            if matches!(event, GameEvent::ProtocolRejected { .. }) {
                return;
            }
        }
    }
    panic!("v4 range mismatch was not rejected");
}

#[test]
fn v4_resume_keeps_selected_version_and_reauthorizes_with_new_handles() {
    let server_key = Keypair::generate().unwrap();
    let client_key = Keypair::generate().unwrap();
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production(),
        ClientSecurity::pinned(client_key, server_key.public.clone()),
    )
    .unwrap();
    let listener = runtime
        .listen_range(GameRangeServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocolRange::new(73, 1, 9),
        })
        .unwrap();
    let address = runtime.endpoint_local_addr(listener).unwrap();
    let client = runtime
        .connect_range(GameRangeClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: address,
            join_ticket: b"first".to_vec(),
            protocol: GameProtocolRange::new(73, 2, 6),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut old_server = None;
    let mut old_client = None;
    while (old_server.is_none() || old_client.is_none()) && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).unwrap()
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                    old_server = Some(session)
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == client => {
                    old_client = Some(session)
                }
                _ => {}
            }
        }
    }
    let old_server = old_server.expect("first server ready");
    let old_client = old_client.expect("first client ready");
    assert_eq!(runtime.selected_protocol_version(old_server).unwrap(), 6);
    runtime.issue_resume_ticket(old_server, b"player").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let ticket = loop {
        assert!(Instant::now() < deadline, "v4 resume ticket not delivered");
        if let Some(ticket) = runtime
            .poll(32, Duration::from_millis(10))
            .into_iter()
            .find_map(|event| {
                if let GameEvent::ResumeTicket { ticket, .. } = event {
                    Some(ticket)
                } else {
                    None
                }
            })
        {
            break ticket;
        }
    };
    let invalid = runtime
        .connect_range_resume(
            GameRangeClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr: address,
                join_ticket: b"wrong-range".to_vec(),
                protocol: GameProtocolRange::new(73, 7, 8),
            },
            old_client,
            ticket.as_bytes(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut rejected = false;
    while !rejected && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            assert!(!matches!(event, GameEvent::ResumeRequest { .. }));
            if matches!(event, GameEvent::ProtocolRejected { .. }) {
                rejected = true;
            }
        }
    }
    assert!(rejected, "out-of-range resume was not rejected");
    assert_eq!(
        runtime.resume_metrics_snapshot().outstanding_tickets,
        1,
        "an invalid range must not consume the ticket"
    );
    runtime.close_endpoint(invalid).unwrap();
    assert_eq!(
        runtime.resume_metrics_snapshot().outstanding_tickets,
        1,
        "closing the failed client endpoint must not revoke the server ticket"
    );
    let resumed_client = runtime
        .connect_range_resume(
            GameRangeClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr: address,
                join_ticket: b"resume".to_vec(),
                protocol: GameProtocolRange::new(73, 4, 8),
            },
            old_client,
            ticket.as_bytes(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut server_mapping = None;
    let mut client_mapping = None;
    while (server_mapping.is_none() || client_mapping.is_none()) && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            match event {
                GameEvent::ResumeRequest {
                    old_session,
                    session,
                    ..
                } => {
                    assert_eq!(old_session, old_server);
                    assert_eq!(runtime.selected_protocol_version(session).unwrap(), 6);
                    runtime.auth_decide(session, true).unwrap();
                }
                GameEvent::SessionResumed {
                    endpoint,
                    old_session,
                    new_session,
                } if endpoint == listener => {
                    assert_eq!(old_session, old_server);
                    server_mapping = Some(new_session);
                }
                GameEvent::SessionResumed {
                    endpoint,
                    old_session,
                    new_session,
                } if endpoint == resumed_client => {
                    assert_eq!(old_session, old_client);
                    client_mapping = Some(new_session);
                }
                _ => {}
            }
        }
    }
    let new_server = server_mapping.expect("server resume mapping");
    let new_client = client_mapping.expect("client resume mapping");
    assert_ne!(old_server, new_server);
    assert_ne!(old_client, new_client);
    assert_eq!(runtime.selected_protocol_version(new_server).unwrap(), 6);
    assert_eq!(runtime.selected_protocol_version(new_client).unwrap(), 6);
    assert_eq!(
        runtime.send(old_client, b"stale").unwrap_err().code(),
        ErrorCode::InvalidHandle
    );
}
