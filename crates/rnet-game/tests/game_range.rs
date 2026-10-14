use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::{Duration, Instant};

#[test]
fn exact_and_range_bounds_share_negotiation_on_every_transport() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        for exact_server in [false, true] {
            let key = Keypair::generate().unwrap();
            let runtime = GameRuntime::new_with_client_security(
                GameRuntimeConfig::production(),
                ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
            )
            .unwrap();
            let listener = if exact_server {
                runtime.listen(GameServerConfig {
                    transport,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: key,
                    initial_encryption: true,
                    protocol: GameProtocol::new(71, 5, 5),
                })
            } else {
                runtime.listen(GameServerConfig {
                    transport,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: key,
                    initial_encryption: true,
                    protocol: GameProtocol::new(71, 3, 8),
                })
            }
            .unwrap();
            let remote_addr = runtime.endpoint_local_addr(listener).unwrap();
            let client = if exact_server {
                runtime.connect(GameClientConfig {
                    transport,
                    bind_addr: None,
                    remote_addr,
                    join_ticket: b"login".to_vec(),
                    protocol: GameProtocol {
                        build_id: 42,
                        capabilities: 9,
                        ..GameProtocol::new(71, 3, 8)
                    },
                })
            } else {
                runtime.connect(GameClientConfig {
                    transport,
                    bind_addr: None,
                    remote_addr,
                    join_ticket: b"login".to_vec(),
                    protocol: GameProtocol {
                        build_id: 42,
                        capabilities: 9,
                        ..GameProtocol::new(71, 5, 5)
                    },
                })
            }
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut server_session, mut client_session) = (None, None);
            let mut authorized = false;
            while (server_session.is_none() || client_session.is_none())
                && Instant::now() < deadline
            {
                for event in runtime.poll(32, Duration::from_millis(10)) {
                    match event {
                        GameEvent::AuthRequest {
                            session,
                            build_id,
                            capabilities,
                            join_ticket,
                            ..
                        } => {
                            assert_eq!((build_id, capabilities), (42, 9));
                            assert_eq!(join_ticket.as_ref(), b"login");
                            assert_eq!(runtime.selected_protocol_version(session).unwrap(), 5);
                            assert_eq!(
                                runtime.send(session, b"early").unwrap_err().code(),
                                ErrorCode::HandshakeRequired
                            );
                            runtime.auth_decide(session, true).unwrap();
                            authorized = true;
                        }
                        GameEvent::SessionReady { endpoint, session } => {
                            assert!(authorized);
                            assert_eq!(runtime.selected_protocol_version(session).unwrap(), 5);
                            if endpoint == listener {
                                server_session = Some(session);
                            } else {
                                assert_eq!(endpoint, client);
                                client_session = Some(session);
                            }
                        }
                        GameEvent::ProtocolRejected { .. }
                        | GameEvent::ProtocolViolation { .. } => {
                            panic!("cross-entry join rejected: {event:?}")
                        }
                        _ => {}
                    }
                }
            }
            let server_session = server_session.expect("server ready");
            runtime
                .send(client_session.expect("client ready"), b"business")
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                assert!(Instant::now() < deadline, "business message not delivered");
                if runtime.poll(32, Duration::from_millis(10)).into_iter().any(|event| {
                    matches!(event, GameEvent::Message(message) if message.session == server_session && message.payload == b"business"[..])
                }) { break; }
            }
        }
    }
}

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
            .listen(GameServerConfig {
                transport,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: server_key,
                initial_encryption: true,
                protocol: GameProtocol::new(71, 3, 8),
            })
            .unwrap();
        let client = runtime
            .connect(GameClientConfig {
                transport,
                bind_addr: None,
                remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
                join_ticket: b"join".to_vec(),
                protocol: GameProtocol::new(71, 5, 10),
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
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(72, 1, 2),
        })
        .unwrap();
    runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_ticket: b"join".to_vec(),
            protocol: GameProtocol::new(72, 3, 4),
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
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(73, 1, 9),
        })
        .unwrap();
    let address = runtime.endpoint_local_addr(listener).unwrap();
    let client = runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: address,
            join_ticket: b"first".to_vec(),
            protocol: GameProtocol::new(73, 2, 6),
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
        .connect_resume(
            GameClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr: address,
                join_ticket: b"wrong-range".to_vec(),
                protocol: GameProtocol::new(73, 7, 8),
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
        .connect_resume(
            GameClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr: address,
                join_ticket: b"resume".to_vec(),
                protocol: GameProtocol::new(73, 4, 8),
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
