use crate::envelope::{encode_control, ControlKind};
use crate::{
    GameEvent, GameProtocolRange, GameRangeClientConfig, GameRangeServerConfig, GameRuntime,
    GameRuntimeConfig,
};
use rnet_core::{ErrorCode, Transport};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::{Duration, Instant};

fn assert_no_failure(event: &GameEvent) {
    assert!(
        !matches!(
            event,
            GameEvent::ProtocolViolation { .. }
                | GameEvent::SessionClosed { .. }
                | GameEvent::JoinFailed { .. }
                | GameEvent::ProtocolRejected { .. }
        ),
        "unexpected game failure: {event:?}"
    );
}

fn verify_client_policy(transport: Transport) {
    let key = Keypair::generate().unwrap();
    let mut config = GameRuntimeConfig::default().allow_plaintext_business_data(true);
    config.network.worker_threads = 1;
    let server = GameRuntime::new(config).unwrap();
    let mut client_config = GameRuntimeConfig::default();
    client_config.network.worker_threads = 1;
    let client = GameRuntime::new_with_client_security(
        client_config,
        ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
    )
    .unwrap();
    let protocol = GameProtocolRange::new(95, 1, 2);
    let listener = server
        .listen_range(GameRangeServerConfig {
            transport,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: key,
            initial_encryption: false,
            protocol,
        })
        .unwrap();
    client
        .connect_range(GameRangeClientConfig {
            transport,
            bind_addr: None,
            remote_addr: server.endpoint_local_addr(listener).unwrap(),
            join_ticket: b"ticket".to_vec(),
            protocol,
        })
        .unwrap();
    let mut sessions = [0; 2];
    let deadline = Instant::now() + Duration::from_secs(3);
    while sessions.contains(&0) {
        assert!(
            Instant::now() < deadline,
            "game ready timeout: {transport:?}"
        );
        for (index, runtime) in [&server, &client].into_iter().enumerate() {
            for event in runtime.poll(32, Duration::from_millis(5)) {
                assert_no_failure(&event);
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        runtime.auth_decide(session, true).unwrap()
                    }
                    GameEvent::SessionReady { session, .. } => sessions[index] = session,
                    _ => {}
                }
            }
        }
    }
    let [server_session, client_session] = sessions;
    // A forged control envelope on the unprotected business channel is not a trusted protocol
    // violation. The receiver must not need the server's plaintext opt-in in its own config.
    let forged = encode_control(ControlKind::Heartbeat, b"", 1024).unwrap();
    server.network.send(server_session, &forged).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while client
        .protocol_metrics_snapshot()
        .plaintext_invalid_envelopes_dropped
        == 0
    {
        assert!(
            Instant::now() < deadline,
            "plaintext frame was not processed"
        );
        for runtime in [&server, &client] {
            for event in runtime.poll(32, Duration::from_millis(5)) {
                assert_no_failure(&event);
                assert!(
                    !matches!(event, GameEvent::Message(_)),
                    "forged envelope escaped"
                );
            }
        }
    }
    assert_eq!(
        client
            .protocol_metrics_snapshot()
            .plaintext_invalid_envelopes_dropped,
        1
    );
    server.send(server_session, b"still usable").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut delivered = false;
    while !delivered {
        assert!(
            Instant::now() < deadline,
            "valid data did not follow discarded frame"
        );
        for event in server.poll(32, Duration::from_millis(5)) {
            assert_no_failure(&event);
        }
        for event in client.poll(32, Duration::from_millis(5)) {
            assert_no_failure(&event);
            if let GameEvent::Message(message) = event {
                assert_eq!(message.session, client_session);
                assert_eq!(message.payload.as_ref(), b"still usable");
                delivered = true;
            }
        }
    }

    server.set_encryption(server_session, true).unwrap();
    let mut changed = [false; 2];
    let deadline = Instant::now() + Duration::from_secs(3);
    while changed.contains(&false) {
        assert!(Instant::now() < deadline, "encryption transition timed out");
        for (index, runtime) in [&server, &client].into_iter().enumerate() {
            for event in runtime.poll(32, Duration::from_millis(5)) {
                assert_no_failure(&event);
                if let GameEvent::SecurityChanged { encrypted, .. } = event {
                    assert!(encrypted);
                    changed[index] = true;
                }
            }
        }
    }
    // Exactly the same malformed bytes must fail closed once integrity is verified. Do not
    // classify from a cached security mode: queued events retain their own integrity evidence.
    server.network.send(server_session, &forged).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(
            Instant::now() < deadline,
            "authenticated violation was not reported"
        );
        server.poll(32, Duration::from_millis(5));
        if client.poll(32, Duration::from_millis(5)).into_iter().any(|event| {
            matches!(event, GameEvent::ProtocolViolation { session, .. } if session == client_session)
        }) { break; }
    }
    assert_eq!(
        client
            .protocol_metrics_snapshot()
            .plaintext_invalid_envelopes_dropped,
        1
    );
    assert_eq!(
        client
            .network
            .validate_payload_len(client_session, 0)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidHandle
    );
}

#[test]
fn default_tcp_game_client_uses_received_integrity_for_bad_envelopes() {
    verify_client_policy(Transport::Tcp);
}

#[test]
fn default_udp_game_client_uses_received_integrity_for_bad_envelopes() {
    verify_client_policy(Transport::Udp);
}

#[test]
fn default_kcp_game_client_uses_received_integrity_for_bad_envelopes() {
    verify_client_policy(Transport::Kcp);
}
