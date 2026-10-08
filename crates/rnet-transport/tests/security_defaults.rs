use rnet_core::{ErrorCode, Event, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientConfig, ClientSecurity, EndpointConfig, NetworkRuntime, RuntimeConfig, SecurityChange,
    SecurityMode, ServerConfig,
};
use std::time::{Duration, Instant};

#[test]
fn default_policy_disables_plaintext_and_legacy_and_enables_automatic_rekey() {
    let policy = RuntimeConfig::default().security_policy;
    assert!(!policy.allow_plaintext_business_data);
    assert!(!policy.allow_legacy_unauthenticated_endpoints);
    assert!(policy
        .rekey_after
        .is_some_and(|duration| !duration.is_zero()));
    assert!(policy.rekey_after_bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(policy, RuntimeConfig::production().security_policy);
}

#[test]
fn default_runtime_rejects_unprotected_endpoints_without_consuming_capacity() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        worker_threads: 1,
        max_endpoints: 1,
        ..RuntimeConfig::default()
    })
    .unwrap();
    let address = "127.0.0.1:0".parse().unwrap();
    for config in [
        EndpointConfig::tcp_listener(address),
        EndpointConfig::tcp_client("127.0.0.1:9".parse().unwrap()),
        EndpointConfig::udp(address, None),
        EndpointConfig::kcp(address),
    ] {
        assert_eq!(
            runtime.open_endpoint(config).unwrap_err().code(),
            ErrorCode::NotSupported
        );
        assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
        assert_eq!(runtime.metrics_snapshot().current_sessions, 0);
    }
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let key = Keypair::generate().unwrap();
        let config = ServerConfig {
            transport,
            bind_addr: address,
            local_key: key,
            initial_security: SecurityMode::Plaintext,
        };
        assert_eq!(
            runtime.listen(config.clone()).unwrap_err().code(),
            ErrorCode::NotSupported
        );
        assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
        let endpoint = runtime
            .listen(ServerConfig {
                initial_security: SecurityMode::Encrypted,
                ..config
            })
            .expect("rejected plaintext listener must not consume the sole endpoint slot");
        runtime.close_endpoint(endpoint).unwrap();
    }
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn plaintext_opt_in_does_not_enable_unauthenticated_endpoints() {
    let mut config = RuntimeConfig::default();
    config.security_policy.allow_plaintext_business_data = true;
    let runtime = NetworkRuntime::new(config).unwrap();
    assert_eq!(
        runtime
            .open_endpoint(EndpointConfig::tcp_listener("127.0.0.1:0".parse().unwrap()))
            .unwrap_err()
            .code(),
        ErrorCode::NotSupported
    );
    assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
}

fn wait_for(runtime: &NetworkRuntime, kind: EventType) -> Event {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        for event in runtime.poll_events(16, Duration::from_millis(10)) {
            assert_ne!(event.event_type, EventType::SessionClosed, "{event:?}");
            assert_ne!(event.event_type, EventType::JoinFailed, "{event:?}");
            if event.event_type == kind {
                return event;
            }
        }
    }
    panic!(
        "timed out waiting for {kind:?}: {:?}",
        runtime.metrics_snapshot()
    );
}

fn connected_pair(
    transport: Transport,
    allow_plaintext: bool,
) -> (NetworkRuntime, NetworkRuntime, u64, u64) {
    let key = Keypair::generate().unwrap();
    let mut config = RuntimeConfig {
        worker_threads: 1,
        ..RuntimeConfig::default()
    };
    config.security_policy.allow_plaintext_business_data = allow_plaintext;
    let server = NetworkRuntime::new(config).unwrap();
    // The client uses the untouched default security policy, even when its server permits
    // plaintext. Only the authenticated server chooses the active business-data mode.
    let client = NetworkRuntime::new_with_client_security(
        RuntimeConfig {
            worker_threads: 1,
            ..RuntimeConfig::default()
        },
        Some(ClientSecurity::pinned(
            Keypair::generate().unwrap(),
            key.public.clone(),
        )),
    )
    .unwrap();
    let listener = server
        .listen(ServerConfig {
            transport,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: key,
            initial_security: SecurityMode::Encrypted,
        })
        .unwrap();
    client
        .connect(ClientConfig {
            transport,
            bind_addr: None,
            remote_addr: server.endpoint_local_addr(listener).unwrap(),
            join_payload: b"authorized-game-client".to_vec(),
        })
        .unwrap();
    let auth = wait_for(&server, EventType::AuthRequest);
    assert_eq!(
        server.send(auth.session, b"early").unwrap_err().code(),
        ErrorCode::HandshakeRequired
    );
    server.auth_decide(auth.session, true).unwrap();
    let server_session = wait_for(&server, EventType::SessionOpened).session;
    let client_session = wait_for(&client, EventType::SessionOpened).session;
    (server, client, server_session, client_session)
}

fn exchange(
    server: &NetworkRuntime,
    client: &NetworkRuntime,
    server_session: u64,
    client_session: u64,
    encrypted: bool,
) {
    client.send(client_session, b"client request").unwrap();
    let request = wait_for(server, EventType::Message);
    assert_eq!(request.session, server_session);
    assert_eq!(request.data, b"client request");
    assert_eq!(request.integrity_verified, encrypted);
    server.send(server_session, b"server reply").unwrap();
    let reply = wait_for(client, EventType::Message);
    assert_eq!(reply.session, client_session);
    assert_eq!(reply.data, b"server reply");
    assert_eq!(reply.integrity_verified, encrypted);
}

#[test]
fn default_server_rejects_downgrade_without_disrupting_encrypted_messages() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let (server, client, server_session, client_session) = connected_pair(transport, false);
        assert_eq!(
            server
                .set_security_mode(server_session, SecurityMode::Plaintext)
                .unwrap_err()
                .code(),
            ErrorCode::NotSupported
        );
        exchange(&server, &client, server_session, client_session, true);
        assert_eq!(server.metrics_snapshot().current_sessions, 1);
        assert_eq!(client.metrics_snapshot().current_sessions, 1);
    }
}

#[test]
fn default_clients_follow_authorized_server_mode_changes_on_all_transports() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let (server, client, server_session, client_session) = connected_pair(transport, true);
        exchange(&server, &client, server_session, client_session, true);
        for mode in [SecurityMode::Plaintext, SecurityMode::Encrypted] {
            server.set_security_mode(server_session, mode).unwrap();
            for runtime in [&server, &client] {
                let changed = wait_for(runtime, EventType::SecurityChanged);
                assert_eq!(SecurityChange::from_event(&changed).unwrap().mode, mode);
            }
            exchange(
                &server,
                &client,
                server_session,
                client_session,
                mode == SecurityMode::Encrypted,
            );
        }
        // Client defaults must not prevent accepting a server-led rekey either.
        server.rekey_session(server_session).unwrap();
        for runtime in [&server, &client] {
            wait_for(runtime, EventType::SecurityChanged);
        }
        exchange(&server, &client, server_session, client_session, true);
    }
}
