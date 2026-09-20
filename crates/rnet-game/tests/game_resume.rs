use rnet_core::{ErrorCode, Event, EventType, Transport};
use rnet_game::{
    GameClientConfig, GameEvent, GameHostClientConfig, GameProtocol, GameRuntime,
    GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::{
    AuthRequest, ClientConfig, ClientSecurity, EndpointConfig, HostClientConfig,
    ResolvedClientConfig,
};
use std::time::{Duration, Instant};

#[test]
fn credentials_are_not_in_debug_output() {
    let secret = b"private-credential-marker".to_vec();
    let secret_debug = format!("{secret:?}");
    let key = Keypair::generate().unwrap();
    let private_debug = format!("{:?}", key.private);
    assert!(!format!("{key:?}").contains(&private_debug));

    let mut event = Event::simple(EventType::AuthRequest);
    event.data = secret.clone();
    assert!(!format!("{event:?}").contains(&secret_debug));
    let auth = AuthRequest {
        client_public_key: [0; 32],
        join_payload: &secret,
    };
    assert!(!format!("{auth:?}").contains(&secret_debug));

    let client = ClientConfig {
        transport: Transport::Tcp,
        bind_addr: None,
        remote_addr: "127.0.0.1:1".parse().unwrap(),
        join_payload: secret.clone(),
    };
    assert!(!format!("{client:?}").contains(&secret_debug));
    let endpoint = EndpointConfig::client(Transport::Tcp, None, client.remote_addr, secret.clone());
    assert!(!format!("{endpoint:?}").contains(&secret_debug));
    let host = HostClientConfig {
        transport: Transport::Tcp,
        host: "localhost".into(),
        port: 1,
        join_payload: secret.clone(),
    };
    assert!(!format!("{host:?}").contains(&secret_debug));
    let resolved = ResolvedClientConfig {
        transport: Transport::Tcp,
        remote_addrs: vec![client.remote_addr],
        join_payload: secret.clone(),
    };
    assert!(!format!("{resolved:?}").contains(&secret_debug));
    let game_client = GameClientConfig {
        transport: Transport::Tcp,
        bind_addr: None,
        remote_addr: client.remote_addr,
        join_ticket: secret.clone(),
        protocol: GameProtocol::new(1, 1),
    };
    assert!(!format!("{game_client:?}").contains(&secret_debug));
    let game_host = GameHostClientConfig {
        transport: Transport::Tcp,
        host: "localhost".into(),
        port: 1,
        join_ticket: secret,
        protocol: GameProtocol::new(1, 1),
    };
    assert!(!format!("{game_host:?}").contains(&secret_debug));
}

#[test]
fn authorized_resume_uses_new_handles_and_rejects_ticket_replay() {
    for _ in 0..5 {
        for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
            exercise_authorized_resume(transport);
        }
    }
}

fn exercise_authorized_resume(transport: Transport) {
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
            protocol: GameProtocol::new(0x71, 2),
        })
        .unwrap();
    let addr = runtime.endpoint_local_addr(listener).unwrap();
    let config = || GameClientConfig {
        transport,
        bind_addr: None,
        remote_addr: addr,
        join_ticket: b"fresh-login-proof".to_vec(),
        protocol: GameProtocol::new(0x71, 2),
    };
    let client = runtime.connect(config()).unwrap();
    let mut server_old = None;
    let mut client_old = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while (server_old.is_none() || client_old.is_none()) && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).unwrap()
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                    server_old = Some(session)
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == client => {
                    client_old = Some(session)
                }
                _ => {}
            }
        }
    }
    let server_old = server_old.expect("old server session");
    let client_old = client_old.expect("old client session");
    runtime
        .issue_resume_ticket(server_old, b"player-71")
        .unwrap();
    let mut ticket = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while ticket.is_none() && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            if let GameEvent::ResumeTicket {
                endpoint,
                ticket: value,
                ..
            } = event
            {
                assert_eq!(endpoint, client);
                ticket = Some(value);
            }
        }
    }
    let ticket = ticket.expect("client received ticket");
    runtime.close_session(client_old).unwrap();
    let resumed_client = if transport == Transport::Tcp {
        runtime
            .connect_host_resume(
                GameHostClientConfig {
                    transport,
                    host: "localhost".to_string(),
                    port: addr.port(),
                    join_ticket: b"fresh-login-proof".to_vec(),
                    protocol: GameProtocol::new(0x71, 2),
                },
                client_old,
                &ticket,
            )
            .unwrap()
    } else {
        runtime
            .connect_resume(config(), client_old, &ticket)
            .unwrap()
    };
    let mut server_new = None;
    let mut client_new = None;
    let mut server_mapping = false;
    let mut client_mapping = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while (!server_mapping || !client_mapping) && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            match event {
                GameEvent::ResumeRequest {
                    endpoint,
                    session,
                    old_session,
                    identity,
                    join_ticket,
                    ..
                } => {
                    assert_eq!(endpoint, listener);
                    assert_eq!(old_session, server_old);
                    assert_eq!(identity, b"player-71");
                    assert_eq!(join_ticket, b"fresh-login-proof");
                    runtime.auth_decide(session, true).unwrap();
                }
                GameEvent::SessionResumed {
                    endpoint,
                    old_session,
                    new_session,
                } if endpoint == listener => {
                    assert_eq!(old_session, server_old);
                    assert_ne!(new_session, server_old);
                    server_new = Some(new_session);
                    server_mapping = true;
                }
                GameEvent::SessionResumed {
                    endpoint,
                    old_session,
                    new_session,
                } if endpoint == resumed_client => {
                    assert_eq!(old_session, client_old);
                    assert_ne!(new_session, client_old);
                    client_new = Some(new_session);
                    client_mapping = true;
                }
                _ => {}
            }
        }
    }
    assert!(server_mapping && client_mapping);
    assert_eq!(
        runtime.send(server_old, b"stale").unwrap_err().code(),
        ErrorCode::InvalidHandle
    );
    runtime.send(server_new.unwrap(), b"ready").unwrap();

    runtime
        .connect_resume(config(), client_old, &ticket)
        .unwrap();
    let mut replay_rejected = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !replay_rejected && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            if let GameEvent::ProtocolRejected { endpoint, .. } = event {
                replay_rejected = endpoint == listener;
            }
        }
    }
    assert!(replay_rejected, "a consumed ticket must not be reusable");

    let server_new = server_new.unwrap();
    runtime
        .issue_resume_ticket(server_new, b"player-71")
        .unwrap();
    let mut next_ticket = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while next_ticket.is_none() && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            if let GameEvent::ResumeTicket {
                endpoint, ticket, ..
            } = event
            {
                if endpoint == resumed_client {
                    next_ticket = Some(ticket);
                }
            }
        }
    }
    let rejected_client = runtime
        .connect_resume(
            config(),
            client_new.unwrap(),
            &next_ticket.expect("second ticket"),
        )
        .unwrap();
    let mut denied = false;
    let mut denied_request = false;
    let mut denial_events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !denied && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            if denial_events.len() < 32 {
                denial_events.push(format!("{event:?}"));
            }
            match event {
                GameEvent::ResumeRequest { session, .. } => {
                    denied_request = true;
                    runtime.auth_decide(session, false).unwrap()
                }
                GameEvent::JoinFailed { endpoint, .. } if endpoint == rejected_client => {
                    denied = true
                }
                GameEvent::SessionResumed { endpoint, .. } if endpoint == rejected_client => {
                    panic!("business denial must not create a new ready session")
                }
                _ => {}
            }
        }
    }
    assert!(
        denied,
        "{transport:?}: request={denied_request}, events={denial_events:?}"
    );
    runtime
        .send(server_new, b"old authorization still active")
        .unwrap();

    runtime
        .issue_resume_ticket(server_new, b"player-71")
        .unwrap();
    let mut kick_ticket = None;
    let deadline = Instant::now() + Duration::from_secs(3);
    while kick_ticket.is_none() && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            if let GameEvent::ResumeTicket {
                endpoint, ticket, ..
            } = event
            {
                if endpoint == resumed_client {
                    kick_ticket = Some(ticket);
                }
            }
        }
    }
    let kicked_client = runtime
        .connect_resume(
            config(),
            client_new.unwrap(),
            &kick_ticket.expect("kick ticket"),
        )
        .unwrap();
    let mut kicked = false;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !kicked && Instant::now() < deadline {
        for event in runtime.poll(32, Duration::from_millis(10)) {
            match event {
                GameEvent::ResumeRequest {
                    session,
                    old_session,
                    ..
                } => {
                    assert_eq!(old_session, server_new);
                    runtime.close_session(server_new).unwrap();
                    assert!(runtime.auth_decide(session, true).is_err());
                    kicked = true;
                }
                GameEvent::SessionResumed { endpoint, .. } if endpoint == kicked_client => {
                    panic!("a kicked player must not regain a ready session")
                }
                _ => {}
            }
        }
    }
    assert!(kicked);
    let resume = runtime.resume_metrics_snapshot();
    assert_eq!(resume.tickets_issued, 3);
    assert_eq!(resume.requests_received, 4);
    assert_eq!(resume.tickets_rejected, 1);
    assert_eq!(resume.sessions_resumed, 1);
    assert_eq!(resume.authorization_denied, 1);
    assert!(resume.pending_revoked >= 1);
    assert!(runtime
        .prometheus_snapshot()
        .contains("rnet_game_resume_sessions_resumed_total 1"));
}
