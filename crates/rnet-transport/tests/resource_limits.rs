use rnet_core::{ErrorCode, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    AdmissionRejectReason, ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig,
    SecurityMode, ServerConfig,
};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[test]
fn runtime_endpoint_limit_recovers_after_close() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        max_endpoints: 1,
        ..RuntimeConfig::default()
    })
    .unwrap();
    let first = runtime
        .listen(ServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: Keypair::generate().unwrap(),
            initial_security: SecurityMode::Encrypted,
        })
        .unwrap();

    let error = runtime
        .listen(ServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: Keypair::generate().unwrap(),
            initial_security: SecurityMode::Encrypted,
        })
        .expect_err("second endpoint must be rejected");
    assert_eq!(error.code(), ErrorCode::WouldBlock);
    let metrics = runtime.metrics_snapshot();
    assert_eq!(metrics.current_endpoints, 1);
    assert_eq!(
        metrics.admission_rejected_by_reason[AdmissionRejectReason::EndpointLimit as usize],
        1
    );

    runtime.close_endpoint(first).unwrap();
    runtime
        .listen(ServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: Keypair::generate().unwrap(),
            initial_security: SecurityMode::Encrypted,
        })
        .expect("capacity must recover after close");
}

#[test]
fn pending_handshake_limit_is_global_and_recovers_after_endpoint_close() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        max_endpoints: 2,
        max_pending_handshakes: 1,
        handshake_timeout: Duration::from_secs(5),
        ..RuntimeConfig::default()
    })
    .unwrap();
    let first = runtime
        .listen(ServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: Keypair::generate().unwrap(),
            initial_security: SecurityMode::Encrypted,
        })
        .unwrap();
    let second = runtime
        .listen(ServerConfig {
            transport: Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: Keypair::generate().unwrap(),
            initial_security: SecurityMode::Encrypted,
        })
        .unwrap();
    let _silent_first = TcpStream::connect(runtime.endpoint_local_addr(first).unwrap()).unwrap();
    wait_until(Duration::from_secs(1), || {
        runtime.metrics_snapshot().pending_handshakes == 1
    });

    let _silent_second = TcpStream::connect(runtime.endpoint_local_addr(second).unwrap()).unwrap();
    wait_until(Duration::from_secs(1), || {
        runtime.metrics_snapshot().admission_rejected > 0
    });
    let metrics = runtime.metrics_snapshot();
    assert_eq!(metrics.pending_handshakes, 1);
    assert_eq!(metrics.peak_pending_handshakes, 1);
    assert!(
        metrics.admission_rejected_by_reason[AdmissionRejectReason::PendingHandshakeLimit as usize]
            > 0
    );

    runtime.close_endpoint(first).unwrap();
    wait_until(Duration::from_secs(1), || {
        runtime.metrics_snapshot().pending_handshakes == 0
    });
}

#[test]
fn datagram_auth_event_backpressure_rolls_back_the_pending_session() {
    let server_key = Keypair::generate().unwrap();
    let server = NetworkRuntime::new(RuntimeConfig {
        event_queue_capacity: 2,
        ..RuntimeConfig::production()
    })
    .unwrap();
    let listener = server
        .listen(ServerConfig {
            transport: Transport::Udp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: server_key.clone(),
            initial_security: SecurityMode::Encrypted,
        })
        .unwrap();
    let server_addr = server.endpoint_local_addr(listener).unwrap();

    let client = NetworkRuntime::new_with_client_security(
        RuntimeConfig::production(),
        Some(ClientSecurity::pinned(
            Keypair::generate().unwrap(),
            server_key.public.clone(),
        )),
    )
    .unwrap();
    client
        .connect(ClientConfig {
            transport: Transport::Udp,
            bind_addr: Some("127.0.0.1:0".parse().unwrap()),
            remote_addr: server_addr,
            join_payload: Vec::new(),
        })
        .unwrap();

    wait_until(Duration::from_secs(2), || {
        server.metrics_snapshot().protocol_errors > 0
    });
    let metrics = server.metrics_snapshot();
    assert_eq!(metrics.current_sessions, 0);
    assert_eq!(metrics.pending_handshakes, 0);
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("condition was not met before timeout");
}
