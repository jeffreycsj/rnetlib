use rnet_core::{ErrorCode, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientSecurity, NetworkRuntime, ResolvedClientConfig, RuntimeConfig, SecurityMode, ServerConfig,
};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

fn runtime_with_timeouts(
    server_key: &Keypair,
    handshake_timeout: Duration,
    connect_timeout: Duration,
) -> NetworkRuntime {
    let security = ClientSecurity::pinned(
        Keypair::generate().expect("client key"),
        server_key.public.clone(),
    );
    NetworkRuntime::new_with_client_security(
        RuntimeConfig {
            handshake_timeout,
            connect_timeout,
            ..RuntimeConfig::default()
        },
        Some(security),
    )
    .expect("runtime")
}

#[test]
fn udp_and_kcp_fail_over_from_ipv4_to_ipv6() {
    for transport in [Transport::Udp, Transport::Kcp] {
        let server_key = Keypair::generate().expect("server key");
        let runtime = runtime_with_timeouts(
            &server_key,
            Duration::from_millis(100),
            Duration::from_secs(2),
        );
        let listener = runtime
            .listen(ServerConfig {
                transport,
                bind_addr: SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
                local_key: server_key,
                initial_security: SecurityMode::Encrypted,
            })
            .expect("IPv6 listener");
        let blackhole = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = runtime
            .connect_resolved(ResolvedClientConfig {
                transport,
                remote_addrs: vec![
                    blackhole.local_addr().unwrap(),
                    runtime.endpoint_local_addr(listener).unwrap(),
                ],
                join_payload: b"cross-family".to_vec(),
            })
            .expect("client endpoint");

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut opened = false;
        while Instant::now() < deadline && !opened {
            for event in runtime.poll_events(32, Duration::from_millis(20)) {
                if event.event_type == EventType::AuthRequest {
                    runtime.auth_decide(event.session, true).unwrap();
                }
                if event.event_type == EventType::SessionOpened && event.endpoint == client {
                    opened = true;
                }
            }
        }
        assert!(opened, "{transport:?} did not switch from IPv4 to IPv6");
    }
}

#[test]
fn datagram_candidates_share_one_connection_deadline() {
    let server_key = Keypair::generate().expect("server key");
    let runtime = runtime_with_timeouts(
        &server_key,
        Duration::from_millis(450),
        Duration::from_millis(650),
    );
    let blackholes: Vec<_> = (0..3)
        .map(|_| std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
        .collect();
    let addresses = blackholes
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect();
    let started = Instant::now();
    let client = runtime
        .connect_resolved(ResolvedClientConfig {
            transport: Transport::Udp,
            remote_addrs: addresses,
            join_payload: Vec::new(),
        })
        .expect("client endpoint");

    let wait_deadline = started + Duration::from_millis(1050);
    let mut failure = None;
    while Instant::now() < wait_deadline && failure.is_none() {
        failure = runtime
            .poll_events(32, Duration::from_millis(20))
            .into_iter()
            .find(|event| event.event_type == EventType::JoinFailed && event.endpoint == client);
    }

    let failure = failure.expect("candidate retries exceeded the overall connection deadline");
    assert_eq!(failure.status, ErrorCode::Timeout);
    assert!(started.elapsed() < Duration::from_millis(1050));
}
