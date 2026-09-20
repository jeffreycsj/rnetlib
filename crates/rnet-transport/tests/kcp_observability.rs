use rnet_core::{ErrorCode, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientSecurity, HostClientConfig, LatencyKind, NetworkRuntime, RuntimeConfig, SecurityMode,
    ServerConfig,
};
use std::time::{Duration, Instant};

#[test]
fn kcp_send_queue_latency_counts_each_accepted_message_once() {
    let server_key = Keypair::generate().expect("server key");
    let client_security = ClientSecurity::pinned(
        Keypair::generate().expect("client key"),
        server_key.public.clone(),
    );
    let runtime = NetworkRuntime::new_with_client_security(
        RuntimeConfig {
            event_queue_capacity: 2048,
            write_queue_capacity: 128,
            max_runtime_queued_bytes: 256 * 1024,
            max_session_queued_bytes: 64 * 1024,
            ..RuntimeConfig::default()
        },
        Some(client_security),
    )
    .expect("runtime");
    let listener = runtime
        .listen(ServerConfig {
            transport: Transport::Kcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key,
            initial_security: SecurityMode::Encrypted,
        })
        .expect("listener");
    let client = runtime
        .connect_host(HostClientConfig {
            transport: Transport::Kcp,
            host: "127.0.0.1".to_owned(),
            port: runtime.endpoint_local_addr(listener).unwrap().port(),
            join_payload: b"metrics-test".to_vec(),
        })
        .expect("client");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut client_session = None;
    while Instant::now() < deadline && client_session.is_none() {
        for event in runtime.poll_events(32, Duration::from_millis(10)) {
            if event.event_type == EventType::AuthRequest {
                runtime.auth_decide(event.session, true).unwrap();
            } else if event.event_type == EventType::SessionOpened && event.endpoint == client {
                client_session = Some(event.session);
            }
        }
    }
    let client_session = client_session.expect("client session");

    const MESSAGE_COUNT: u64 = 256;
    let payload = vec![7_u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut accepted = 0_u64;
    let mut received = 0_u64;
    while received < MESSAGE_COUNT && Instant::now() < deadline {
        while accepted < MESSAGE_COUNT {
            match runtime.send(client_session, 1, &payload) {
                Ok(()) => accepted += 1,
                Err(error) if error.code() == ErrorCode::WouldBlock => break,
                Err(error) => panic!("send failed: {error}"),
            }
        }
        received += runtime
            .poll_events(512, Duration::from_millis(5))
            .into_iter()
            .filter(|event| event.event_type == EventType::Message && event.endpoint == listener)
            .count() as u64;
    }
    assert_eq!(accepted, MESSAGE_COUNT);
    assert_eq!(received, MESSAGE_COUNT);

    let send_queue = runtime
        .latency_snapshot()
        .into_iter()
        .find(|metric| metric.kind == LatencyKind::SendQueue)
        .expect("send queue latency");
    assert_eq!(send_queue.latency.sample_count, MESSAGE_COUNT);
}
