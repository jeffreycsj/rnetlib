use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

#[test]
fn closing_endpoint_releases_ready_state_and_pending_snapshots_without_another_poll() {
    let server_key = Keypair::generate().unwrap();
    let client_key = Keypair::generate().unwrap();
    let runtime = Arc::new(
        GameRuntime::new_with_client_security(
            GameRuntimeConfig::production(),
            ClientSecurity::pinned(client_key, server_key.public.clone()),
        )
        .unwrap(),
    );
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(88, 1),
        })
        .unwrap();
    runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_ticket: b"ticket".to_vec(),
            protocol: GameProtocol::new(88, 1),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut server_session = None;
    while server_session.is_none() && Instant::now() < deadline {
        for event in runtime.poll(16, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).unwrap()
                }
                GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                    server_session = Some(session)
                }
                _ => {}
            }
        }
    }
    let session = server_session.expect("server session ready");
    runtime.send_latest(session, 1, b"unsent snapshot").unwrap();
    assert_eq!(runtime.realtime_queue_snapshot().queued_messages, 1);
    let barrier = Arc::new(Barrier::new(2));
    let producer = {
        let runtime = Arc::clone(&runtime);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..1_000 {
                if runtime.send_latest(session, 1, b"new snapshot").is_err() {
                    break;
                }
            }
        })
    };
    barrier.wait();
    runtime.close_endpoint(listener).unwrap();
    producer.join().unwrap();
    assert_eq!(runtime.realtime_queue_snapshot().queued_messages, 0);
    assert!(runtime.realtime_queue_snapshot().closed_dropped >= 1);
    assert_eq!(
        runtime.network_quality(session).unwrap_err().code(),
        ErrorCode::InvalidHandle
    );
}
