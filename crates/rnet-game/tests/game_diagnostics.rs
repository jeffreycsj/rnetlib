use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    BoundedLogger, GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig,
    GameServerConfig, LoggerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn optional_game_logger_records_auth_and_denial_without_credentials() {
    let (sender, receiver) = mpsc::channel();
    let logger = BoundedLogger::new(LoggerConfig::default(), move |record| {
        let _ = sender.send(record);
    })
    .unwrap();
    let server_key = Keypair::generate().unwrap();
    let client_key = Keypair::generate().unwrap();
    let runtime = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production(),
        ClientSecurity::pinned(client_key, server_key.public.clone()),
    )
    .unwrap()
    .with_logger(logger);
    let listener = runtime
        .listen(GameServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocol::new(42, 3),
        })
        .unwrap();
    let client = runtime
        .connect(GameClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_ticket: b"private-login-proof".to_vec(),
            protocol: GameProtocol::new(42, 3),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut denied = false;
    while !denied && Instant::now() < deadline {
        for event in runtime.poll(16, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    runtime.auth_decide(session, false).unwrap();
                }
                GameEvent::JoinFailed {
                    endpoint, reason, ..
                } if endpoint == client => {
                    assert_eq!(reason, ErrorCode::AuthRejected);
                    denied = true;
                }
                _ => {}
            }
        }
    }
    assert!(denied, "client must observe the rejected join");

    let mut names = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while names.len() < 2 && Instant::now() < deadline {
        if let Ok(record) = receiver.recv_timeout(Duration::from_millis(20)) {
            assert!(!record.message.contains("private-login-proof"));
            assert!(record.runtime != 0);
            assert_eq!(record.target, "rnet.game");
            if record.event_name == "game_auth_requested" {
                assert_eq!(record.endpoint, listener);
                assert_eq!(record.transport, Transport::Tcp as u32);
                assert!(record.message.contains("protocol_id=42"));
                names.push(record.event_name);
            } else if record.event_name == "game_join_failed" && record.endpoint == client {
                assert_eq!(record.error_code, ErrorCode::AuthRejected);
                assert!(
                    record.message.contains("authentication rejected"),
                    "lost handshake error detail: {}",
                    record.message
                );
                names.push(record.event_name);
            }
        }
    }
    assert!(names.iter().any(|name| name == "game_auth_requested"));
    assert!(names.iter().any(|name| name == "game_join_failed"));
    let logger = runtime.game_logger_snapshot().expect("logger configured");
    assert_eq!(logger.dropped, 0);
    assert_eq!(logger.sink_panics, 0);
    assert!(runtime.game_logger_callback_latency().unwrap().sample_count >= 2);
    let prometheus = runtime.prometheus_snapshot();
    assert!(prometheus.contains("rnet_game_logger_enabled 1"));
    assert!(prometheus.contains("rnet_game_logger_dropped_total 0"));
    assert!(prometheus.contains("rnet_game_logger_sink_panics_total 0"));
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn logger_remains_optional_for_a_normal_game_runtime() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    assert_eq!(runtime.game_logger_snapshot(), None);
    assert!(runtime
        .prometheus_snapshot()
        .contains("rnet_game_logger_enabled 0"));
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn game_poll_logs_bounded_cumulative_latency_summaries_and_can_disable_them() {
    let (sender, receiver) = mpsc::channel();
    let logger = BoundedLogger::new(LoggerConfig::default(), move |record| {
        let _ = sender.send(record);
    })
    .unwrap();
    let runtime = GameRuntime::new(GameRuntimeConfig::production())
        .unwrap()
        .with_logger(logger);
    runtime
        .set_metrics_log_interval(Duration::from_millis(1))
        .unwrap();
    std::thread::sleep(Duration::from_millis(5));
    runtime.poll(0, Duration::ZERO);
    let deadline = Instant::now() + Duration::from_secs(2);
    let summary = loop {
        let record = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if record.event_name == "game_latency_summary" {
            break record;
        }
    };
    for field in [
        "scope=cumulative",
        "kind=heartbeat_rtt",
        "kind=send_queue",
        "kind=scheduled_queue",
        "count=",
        "p95_us=",
        "p99_us=",
        "p999_us=",
        "logger_dropped=",
    ] {
        assert!(
            summary.message.contains(field),
            "missing {field}: {}",
            summary.message
        );
    }
    runtime.set_metrics_log_interval(Duration::ZERO).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    runtime.poll(0, Duration::ZERO);
    while let Ok(record) = receiver.recv_timeout(Duration::from_millis(30)) {
        assert_ne!(record.event_name, "game_latency_summary");
    }
    assert_eq!(
        runtime
            .set_metrics_log_interval(Duration::MAX)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidArgument
    );
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn sink_panic_is_counted_without_interrupting_game_poll() {
    let logger = BoundedLogger::new(LoggerConfig::default(), |_| {
        panic!("test sink failure");
    })
    .unwrap();
    let runtime = GameRuntime::new(GameRuntimeConfig::production())
        .unwrap()
        .with_logger(logger);
    assert!(runtime
        .poll(16, Duration::from_millis(100))
        .iter()
        .any(|event| matches!(event, GameEvent::RuntimeStarted)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime
        .game_logger_snapshot()
        .is_some_and(|snapshot| snapshot.sink_panics == 0)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(runtime.game_logger_snapshot().unwrap().sink_panics >= 1);
    runtime.stop(Duration::ZERO).unwrap();
}
