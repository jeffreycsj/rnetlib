use super::run_adaptive_datagram;
use crate::config::EndpointSecurity;
use crate::state::SessionTarget;
use crate::{NetworkRuntime, RuntimeConfig};
use rnet_core::{ErrorCode, EventType, Transport};
use rnet_protocol::control::SecurityMode;
use rnet_security::Keypair;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

#[test]
fn lost_open_notification_does_not_leave_a_client_endpoint_without_a_session() {
    for kind in [Transport::Udp, Transport::Kcp] {
        let key = Keypair::generate().unwrap();
        let server = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
        let client = NetworkRuntime::new_with_client_security(
            RuntimeConfig {
                event_queue_capacity: 2,
                ..RuntimeConfig::production()
            },
            Some(crate::ClientSecurity::pinned(
                Keypair::generate().unwrap(),
                key.public.clone(),
            )),
        )
        .unwrap();
        let listener = server
            .listen(crate::ServerConfig {
                transport: kind,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: key,
                initial_security: SecurityMode::Encrypted,
            })
            .unwrap();
        client.poll_events(8, Duration::ZERO);
        let endpoint = client
            .connect(crate::ClientConfig {
                transport: kind,
                bind_addr: Some("127.0.0.1:0".parse().unwrap()),
                remote_addr: server.endpoint_local_addr(listener).unwrap(),
                join_payload: Vec::new(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut authorization = None;
        while authorization.is_none() && std::time::Instant::now() < deadline {
            authorization = server
                .poll_events(32, Duration::from_millis(10))
                .into_iter()
                .find(|event| event.event_type == EventType::AuthRequest);
        }
        let authorization = authorization.expect("server did not request authorization");
        while client
            .shared
            .events
            .try_push(rnet_core::Event::simple(EventType::RuntimeStarted))
            .is_ok()
        {}
        server.auth_decide(authorization.session, true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while client.metrics_snapshot().current_sessions != 0
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(client.metrics_snapshot().current_sessions, 0);
        // Session removal happens just before owner invalidation in this overload path.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while client.endpoint_local_addr(endpoint).is_ok() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            client.endpoint_local_addr(endpoint).err().map(|e| e.code()),
            Some(ErrorCode::InvalidHandle)
        );
        assert_eq!(client.metrics_snapshot().pending_handshakes, 0);
        assert!(client.metrics_snapshot().events_dropped > 0);
    }
}

// Linux delivers loopback ICMP port-unreachable to a connected UDP socket's receive call.
// This exercises the actual endpoint loop without unsafe FD invalidation or production hooks.
#[test]
fn receive_failure_reclaims_only_its_endpoint_even_when_events_are_full() {
    for kind in [Transport::Udp, Transport::Kcp] {
        for full_queue in [false, true] {
            let mut config = RuntimeConfig::production();
            config.max_endpoints = 2;
            config.event_queue_capacity = if full_queue { 1 } else { 16 };
            let runtime = NetworkRuntime::new(config).unwrap();
            if !full_queue {
                runtime.poll_events(16, Duration::ZERO);
            }
            let (endpoint, established, pending, other) = runtime.runtime.block_on(async {
                let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
                let endpoint = runtime
                    .insert_endpoint(socket.local_addr().unwrap(), kind)
                    .unwrap();
                let other = runtime
                    .insert_endpoint("127.0.0.1:1".parse().unwrap(), kind)
                    .unwrap();
                let (sender, receiver) = mpsc::channel(8);
                let (cleanup, cleanup_rx) = mpsc::unbounded_channel();
                let peer = "127.0.0.1:2".parse().unwrap();
                let target = || match kind {
                    Transport::Kcp => SessionTarget::Kcp {
                        sender: sender.clone(),
                        peer,
                        cleanup: Some(cleanup.clone()),
                    },
                    _ => SessionTarget::Udp {
                        sender: sender.clone(),
                        peer,
                        cleanup: Some(cleanup.clone()),
                        max_frame_len: 1200,
                    },
                };
                let established = runtime.insert_session(endpoint, target());
                let pending = runtime
                    .insert_pending_session(endpoint, target(), None, true)
                    .unwrap();
                runtime
                    .send_payload(established, b"queued private payload")
                    .unwrap();
                assert!(runtime.metrics_snapshot().queued_send_bytes > 0);
                let unused = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                socket.connect(unused.local_addr().unwrap()).await.unwrap();
                drop(unused);
                socket.send(b"trigger local ICMP").await.unwrap();
                tokio::time::timeout(
                    Duration::from_secs(3),
                    run_adaptive_datagram(
                        runtime.shared.clone(),
                        endpoint,
                        socket,
                        sender,
                        receiver,
                        cleanup,
                        cleanup_rx,
                        None,
                        None,
                        EndpointSecurity::AdaptiveServer {
                            local_key: Keypair::generate().unwrap(),
                            initial_mode: SecurityMode::Encrypted,
                        },
                        None,
                        kind,
                        Vec::new(),
                        None,
                    ),
                )
                .await
                .expect("receive failure did not exit the endpoint loop");
                (endpoint, established, pending, other)
            });
            assert_eq!(
                runtime.endpoint_local_addr(endpoint).unwrap_err().code(),
                ErrorCode::InvalidHandle
            );
            assert!(runtime.endpoint_local_addr(other).is_ok());
            for session in [established, pending] {
                assert_eq!(
                    runtime.validate_payload_len(session, 0).unwrap_err().code(),
                    ErrorCode::InvalidHandle
                );
            }
            let metrics = runtime.metrics_snapshot();
            assert_eq!(metrics.current_endpoints, 1);
            assert_eq!(metrics.current_sessions, 0);
            assert_eq!(metrics.pending_handshakes, 0);
            assert_eq!(metrics.queued_send_bytes, 0);
            assert_eq!(metrics.closed_sessions(ErrorCode::IoError), 2);
            let events = runtime.poll_events(16, Duration::ZERO);
            if full_queue {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].event_type, EventType::RuntimeStarted);
                assert!(metrics.lifecycle_events_rejected >= 3);
            } else {
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.event_type == EventType::EndpointError)
                        .count(),
                    1
                );
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| event.event_type == EventType::SessionClosed)
                        .count(),
                    2
                );
                for event in &events {
                    assert_eq!(event.endpoint, endpoint);
                    assert_eq!(event.status, ErrorCode::IoError);
                    let detail = std::str::from_utf8(&event.data).unwrap();
                    assert!(detail.contains("phase=datagram_receive"), "{detail}");
                    assert!(!detail.contains("private payload"));
                }
            }
            assert!(runtime
                .insert_endpoint("127.0.0.1:0".parse().unwrap(), kind)
                .is_ok());
            runtime.stop(Duration::ZERO).unwrap();
        }
    }
}
