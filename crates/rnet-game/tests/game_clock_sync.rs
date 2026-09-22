use rnet_core::Transport;
use rnet_game::{
    GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::{Duration, Instant};

#[test]
fn clients_sample_protected_clock_offset_on_all_transports() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let server_key = Keypair::generate().expect("server key");
        let client_key = Keypair::generate().expect("client key");
        let policy = GameRuntimeConfig::production()
            .with_heartbeat(Duration::from_millis(20), Duration::from_millis(500));
        let server = GameRuntime::new(policy.clone()).expect("server runtime");
        std::thread::sleep(Duration::from_millis(25));
        let client = GameRuntime::new_with_client_security(
            policy,
            ClientSecurity::pinned(client_key, server_key.public.clone()),
        )
        .expect("client runtime");
        let listener = server
            .listen(GameServerConfig {
                transport,
                bind_addr: "127.0.0.1:0".parse().expect("address"),
                local_key: server_key,
                initial_encryption: true,
                protocol: GameProtocol::new(99, 1),
            })
            .expect("listener");
        client
            .connect(GameClientConfig {
                transport,
                bind_addr: None,
                remote_addr: server.endpoint_local_addr(listener).expect("address"),
                join_ticket: b"ticket".to_vec(),
                protocol: GameProtocol::new(99, 1),
            })
            .expect("connect");
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut server_session = None;
        let mut client_session = None;
        while (server_session.is_none() || client_session.is_none()) && Instant::now() < deadline {
            for event in server.poll(16, Duration::from_millis(5)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        server.auth_decide(session, true).expect("authorize");
                    }
                    GameEvent::SessionReady { session, .. } => server_session = Some(session),
                    _ => {}
                }
            }
            for event in client.poll(16, Duration::from_millis(5)) {
                if let GameEvent::SessionReady { session, .. } = event {
                    client_session = Some(session);
                }
            }
        }
        let server_session = server_session.expect("server ready");
        let client_session = client_session.expect("client ready");
        let mut sample = None;
        while sample.is_none() && Instant::now() < deadline {
            server.poll(16, Duration::from_millis(5));
            client.poll(16, Duration::from_millis(5));
            sample = client
                .clock_sync_snapshot(client_session)
                .expect("clock snapshot");
        }
        let sample = sample.expect("client clock sample");
        assert!(sample.samples >= 1);
        assert!(client.clock_sync_metrics_snapshot().samples >= 1);
        assert!(server.clock_sync_metrics_snapshot().replies_sent >= 1);
        assert!(client
            .prometheus_snapshot()
            .contains("rnet_game_clock_samples_total"));
        assert!(sample.rtt < Duration::from_millis(500));
        let server_clock = server.clock_micros();
        let client_clock = client.clock_micros();
        let observed_offset = i128::from(server_clock) - i128::from(client_clock);
        assert!(
            (i128::from(sample.server_minus_client_us) - observed_offset).abs() < 200_000,
            "{transport:?}: offset={} observed={observed_offset}",
            sample.server_minus_client_us
        );
        assert!(server
            .clock_sync_snapshot(server_session)
            .expect("server snapshot")
            .is_none());

        client
            .send(client_session, b"game-still-works")
            .expect("send");
        // Clock sampling and session setup consume the earlier shared deadline. Give the
        // independent business-data assertion its own bounded delivery window.
        let message_deadline = Instant::now() + Duration::from_secs(3);
        let mut received = false;
        while !received && Instant::now() < message_deadline {
            client.poll(16, Duration::ZERO);
            received = server
                .poll(16, Duration::from_millis(5))
                .iter()
                .any(|event| matches!(event, GameEvent::Message(message) if message.session == server_session && message.payload.as_ref() == b"game-still-works"));
        }
        assert!(received, "{transport:?}: business message missing");
        client
            .close_session(client_session)
            .expect("close client session");
        assert!(client.clock_sync_snapshot(client_session).is_err());
        server.stop(Duration::ZERO).expect("server stop");
        client.stop(Duration::ZERO).expect("client stop");
    }
}
