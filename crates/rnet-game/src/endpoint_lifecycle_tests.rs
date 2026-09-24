use crate::resume::ResumeScope;
use crate::{
    GameClientConfig, GameEvent, GameProtocol, GameProtocolRange, GameRangeServerConfig,
    GameRuntime, GameRuntimeConfig, GameSendOptions, GameServerConfig,
};
use rnet_core::{ErrorCode, Transport};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::{Duration, Instant};

#[test]
fn failed_datagram_join_is_reaped_even_when_only_zero_capacity_game_poll_runs() {
    for transport in [Transport::Udp, Transport::Kcp] {
        let key = Keypair::generate().unwrap();
        let mut config = GameRuntimeConfig::production();
        config.network.handshake_timeout = Duration::from_millis(100);
        config.network.connect_timeout = Duration::from_millis(200);
        let runtime = GameRuntime::new_with_client_security(
            config,
            ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
        )
        .unwrap();
        let blackhole = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = runtime
            .connect(GameClientConfig {
                transport,
                bind_addr: None,
                remote_addr: blackhole.local_addr().unwrap(),
                join_ticket: Vec::new(),
                protocol: GameProtocol::new(77, 1),
            })
            .unwrap();
        assert!(runtime
            .endpoint_transports
            .lock()
            .unwrap()
            .contains_key(&endpoint));
        let deadline = Instant::now() + Duration::from_secs(3);
        while runtime.network.endpoint_local_addr(endpoint).is_ok() && Instant::now() < deadline {
            // Discard all native notifications before they can reach the game facade.
            runtime.network.poll_events(64, Duration::from_millis(10));
        }
        assert!(runtime.network.endpoint_local_addr(endpoint).is_err());
        runtime.network.poll_events(64, Duration::ZERO);
        assert!(runtime.poll(0, Duration::ZERO).is_empty());
        assert!(!runtime
            .endpoint_transports
            .lock()
            .unwrap()
            .contains_key(&endpoint));
        assert!(!runtime
            .resume
            .lock()
            .unwrap()
            .client_endpoints
            .contains_key(&endpoint));
        runtime.stop(Duration::ZERO).unwrap();
    }
}

#[test]
fn lost_endpoint_notifications_do_not_retain_game_state_or_pending_authorization() {
    for (accepted, rejected) in [(false, false), (false, true), (true, false)] {
        for capacity in [0, 8] {
            let key = Keypair::generate().unwrap();
            let runtime = GameRuntime::new_with_client_security(
                GameRuntimeConfig::production(),
                ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
            )
            .unwrap();
            let protocol = GameProtocol::new(77, 1);
            let listener = runtime
                .listen(GameServerConfig {
                    transport: Transport::Udp,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: key.clone(),
                    initial_encryption: true,
                    protocol,
                })
                .unwrap();
            let other = runtime
                .listen(GameServerConfig {
                    transport: Transport::Udp,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: key,
                    initial_encryption: true,
                    protocol,
                })
                .unwrap();
            runtime
                .connect(GameClientConfig {
                    transport: Transport::Udp,
                    bind_addr: None,
                    remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
                    join_ticket: b"private ticket".to_vec(),
                    protocol,
                })
                .unwrap();
            let (mut session, mut ready) = (0, false);
            let deadline = Instant::now() + Duration::from_secs(5);
            while (session == 0 || (accepted && !ready)) && Instant::now() < deadline {
                for event in runtime.poll(32, Duration::from_millis(10)) {
                    match event {
                        GameEvent::AuthRequest {
                            session: handle, ..
                        } => {
                            session = handle;
                            if accepted {
                                runtime.auth_decide(session, true).unwrap();
                            } else if rejected {
                                runtime.auth_decide(session, false).unwrap();
                            }
                        }
                        GameEvent::SessionReady { endpoint, .. } if endpoint == listener => {
                            ready = true
                        }
                        _ => {}
                    }
                }
            }
            assert_ne!(session, 0);
            assert!(!accepted || ready);
            if rejected {
                assert!(
                    !runtime
                        .session_endpoints
                        .lock()
                        .unwrap()
                        .contains_key(&session),
                    "rejected authorization retained pending endpoint ownership"
                );
            }
            let ticket = runtime
                .resume
                .lock()
                .unwrap()
                .tickets
                .issue(
                    ResumeScope {
                        endpoint: listener,
                        session,
                        peer_key: [7; 32],
                        protocol,
                    },
                    b"identity",
                    Instant::now(),
                    Duration::from_secs(30),
                )
                .unwrap();
            if accepted {
                runtime.send_latest(session, 1, b"snapshot").unwrap();
                runtime
                    .send_scheduled(session, b"scheduled", GameSendOptions::default())
                    .unwrap();
                assert!(runtime.admission.contains(session));
            }
            // Transport invalidation is the authority. Deliberately lose every notification;
            // the facade must not depend on a specific event fitting its bounded queue.
            runtime.network.close_endpoint(listener).unwrap();
            runtime.network.poll_events(4096, Duration::ZERO);
            runtime.poll(capacity, Duration::ZERO);
            assert!(!runtime
                .server_protocols
                .lock()
                .unwrap()
                .contains_key(&listener));
            assert!(!runtime
                .endpoint_transports
                .lock()
                .unwrap()
                .contains_key(&listener));
            assert!(!runtime
                .session_endpoints
                .lock()
                .unwrap()
                .contains_key(&session));
            assert!(!runtime
                .resume
                .lock()
                .unwrap()
                .server_sessions
                .contains_key(&session));
            assert!(!runtime
                .heartbeat_trackers
                .lock()
                .unwrap()
                .contains_key(&session));
            assert!(!runtime
                .clock_trackers
                .lock()
                .unwrap()
                .contains_key(&session));
            assert!(!runtime.udp_sessions.lock().unwrap().contains_key(&session));
            assert!(!runtime.admission.contains(session));
            assert_eq!(runtime.realtime_queue_snapshot().queued_messages, 0);
            assert_eq!(runtime.scheduled_queue_snapshot().queued_messages, 0);
            assert!(runtime
                .resume
                .lock()
                .unwrap()
                .tickets
                .consume(&ticket, listener, [7; 32], protocol, Instant::now(),)
                .is_err());
            assert!(runtime.endpoint_local_addr(other).is_ok());
            assert!(runtime
                .server_protocols
                .lock()
                .unwrap()
                .contains_key(&other));
            assert_eq!(
                runtime.send(session, b"stale").unwrap_err().code(),
                ErrorCode::InvalidHandle
            );
        }
    }
}

#[test]
fn lost_empty_range_endpoint_notification_releases_its_policy() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    let endpoint = runtime
        .listen_range(GameRangeServerConfig {
            transport: Transport::Kcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: Keypair::generate().unwrap(),
            initial_encryption: true,
            protocol: GameProtocolRange::new(99, 1, 2),
        })
        .unwrap();
    runtime.network.close_endpoint(endpoint).unwrap();
    runtime.network.poll_events(4096, Duration::ZERO);
    runtime.poll(0, Duration::ZERO);
    assert!(!runtime
        .range
        .lock()
        .unwrap()
        .server_endpoints
        .contains_key(&endpoint));
    assert!(!runtime
        .endpoint_transports
        .lock()
        .unwrap()
        .contains_key(&endpoint));
}
