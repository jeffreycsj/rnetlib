use super::*;
use crate::{GameClientConfig, GameProtocol, GameRuntimeConfig, GameServerConfig};
use rnet_core::Transport;
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;

#[test]
fn full_public_backlog_still_drives_heartbeat_and_negotiation_deadlines() {
    use crate::heartbeat::HeartbeatTracker;
    use crate::range_state::{Phase, RangeSession};
    let mut config = GameRuntimeConfig::production()
        .with_heartbeat(Duration::from_millis(10), Duration::from_millis(100));
    config.network.event_queue_capacity = 1;
    let runtime = GameRuntime::new(config).unwrap();
    runtime.poll(0, Duration::ZERO);
    assert_eq!(runtime.range.lock().unwrap().public_capacity().0, 0);
    let old = Instant::now() - Duration::from_secs(1);
    let mut tracker =
        HeartbeatTracker::new(Duration::from_millis(10), Duration::from_millis(100), old).unwrap();
    tracker
        .mark_sent(7, old + Duration::from_millis(10))
        .unwrap();
    runtime
        .heartbeat_trackers
        .lock()
        .unwrap()
        .insert(42, tracker);
    runtime.range.lock().unwrap().sessions.insert(
        43,
        RangeSession {
            endpoint: 1,
            protocol: GameProtocol::new(1, 1, 1),
            nonce: [1; 16],
            selected: None,
            server_handle: 0,
            old_session: None,
            server_resume: false,
            phase: Phase::ClientAwaitSelect,
            deadline: old,
            next_retry: old,
            buffered: Default::default(),
            buffered_bytes: 0,
        },
    );
    assert!(runtime.poll(0, Duration::from_secs(1)).is_empty());
    assert_eq!(runtime.heartbeat_metrics_snapshot().timeouts, 1);
    assert!(!runtime.range.lock().unwrap().sessions.contains_key(&43));
    assert!(matches!(
        runtime.poll(1, Duration::ZERO).as_slice(),
        [GameEvent::RuntimeStarted]
    ));
}

#[test]
fn sustained_zero_capacity_poll_bounds_count_and_resumes_without_losing_events() {
    let mut config = GameRuntimeConfig::production();
    config.network.event_queue_capacity = 2;
    let runtime = GameRuntime::new(config).unwrap();
    let key = Keypair::generate().unwrap();
    let mut admitted = Vec::new();
    for _ in 0..16 {
        runtime.poll(0, Duration::ZERO);
        assert!(runtime.range.lock().unwrap().completed_len_for_test() <= 2);
        match runtime.listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: key.clone(),
            initial_encryption: true,
            protocol: GameProtocol::new(123, 1, 1),
        }) {
            Ok(endpoint) => {
                admitted.push(endpoint);
                runtime.close_endpoint(endpoint).unwrap();
            }
            Err(error) => assert_eq!(error.code(), ErrorCode::WouldBlock),
        }
    }
    assert!(!admitted.is_empty());
    let mut opened = Vec::new();
    let mut started = 0;
    for _ in 0..32 {
        for event in runtime.poll(1, Duration::ZERO) {
            match event {
                GameEvent::RuntimeStarted => started += 1,
                GameEvent::EndpointOpened { endpoint } => opened.push(endpoint),
                event => panic!("unexpected event: {event:?}"),
            }
        }
    }
    assert_eq!(started, 1);
    assert_eq!(
        opened, admitted,
        "accepted events must remain FIFO and unique"
    );
    assert_eq!(runtime.range.lock().unwrap().completed_len_for_test(), 0);
    let endpoint = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: key,
            initial_encryption: true,
            protocol: GameProtocol::new(123, 1, 1),
        })
        .unwrap();
    assert!(matches!(runtime.poll(1, Duration::ZERO).as_slice(),
        [GameEvent::EndpointOpened { endpoint: actual }] if *actual == endpoint));
}

#[test]
fn zero_capacity_poll_bounds_payload_bytes_and_draining_restores_delivery() {
    let key = Keypair::generate().unwrap();
    let mut config = GameRuntimeConfig::production()
        .with_heartbeat(Duration::from_secs(60), Duration::from_secs(180));
    config.network.event_queue_capacity = 16;
    config.network.max_event_bytes = 256;
    let runtime = GameRuntime::new_with_client_security(
        config,
        ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
    )
    .unwrap();
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: key,
            initial_encryption: true,
            protocol: GameProtocol::new(124, 1, 1),
        })
        .unwrap();
    runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_ticket: Vec::new(),
            protocol: GameProtocol::new(124, 1, 1),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut sessions = Vec::new();
    while sessions.len() < 2 {
        assert!(Instant::now() < deadline);
        for event in runtime.poll(16, Duration::from_millis(5)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, true).unwrap()
                }
                GameEvent::SessionReady { session, .. } => sessions.push(session),
                _ => {}
            }
        }
    }
    let initial = runtime.network.metrics_snapshot().frames_received;
    for index in 0..3u8 {
        runtime.send(sessions[0], &[index; 100]).unwrap();
        runtime.flush_scheduled(16);
        let deadline = Instant::now() + Duration::from_secs(2);
        while runtime.network.metrics_snapshot().frames_received < initial + u64::from(index) + 1
            || runtime.network.metrics_snapshot().queued_event_bytes < 100
        {
            assert!(
                Instant::now() < deadline,
                "transport did not receive the message"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        for _ in 0..16 {
            runtime.poll(0, Duration::ZERO);
        }
        assert!(
            runtime.range.lock().unwrap().completed_len_for_test() <= 2,
            "three 100-byte messages must not fit a 256-byte staging budget"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut received = Vec::new();
    while received.len() < 3 {
        assert!(
            Instant::now() < deadline,
            "draining did not release backpressure"
        );
        for event in runtime.poll(1, Duration::from_millis(5)) {
            if let GameEvent::Message(message) = event {
                received.push(message.payload);
            }
        }
    }
    for (index, payload) in received.iter().enumerate() {
        assert_eq!(payload.as_ref(), &[index as u8; 100]);
    }
    assert_eq!(runtime.range.lock().unwrap().completed_len_for_test(), 0);
}
