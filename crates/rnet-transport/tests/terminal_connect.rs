use rnet_core::{ErrorCode, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{ClientSecurity, NetworkRuntime, ResolvedClientConfig, RuntimeConfig};
use std::net::UdpSocket;
use std::time::{Duration, Instant};

fn failed_attempts_release_capacity(single_candidate: bool) {
    for transport in [Transport::Udp, Transport::Kcp] {
        let key = Keypair::generate().unwrap();
        let runtime = NetworkRuntime::new_with_client_security(
            RuntimeConfig {
                max_endpoints: 1,
                handshake_timeout: Duration::from_millis(100),
                connect_timeout: Duration::from_millis(160),
                ..RuntimeConfig::production()
            },
            Some(ClientSecurity::pinned(
                Keypair::generate().unwrap(),
                key.public.clone(),
            )),
        )
        .unwrap();
        // Bound but unread sockets absorb hello packets without generating an ICMP error.
        let blackholes: Vec<_> = (0..if single_candidate { 1 } else { 3 })
            .map(|_| UdpSocket::bind("127.0.0.1:0").unwrap())
            .collect();
        for _ in 0..3 {
            runtime.poll_events(64, Duration::ZERO);
            let endpoint = runtime
                .connect_resolved(ResolvedClientConfig {
                    transport,
                    remote_addrs: blackholes.iter().map(|s| s.local_addr().unwrap()).collect(),
                    join_payload: Vec::new(),
                })
                .expect("a failed attempt must return the sole endpoint slot");
            let mut events = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                events.extend(runtime.poll_events(64, Duration::from_millis(10)));
                if events
                    .iter()
                    .any(|e| e.event_type == EventType::SessionClosed)
                {
                    break;
                }
            }
            assert_eq!(
                runtime
                    .endpoint_local_addr(endpoint)
                    .err()
                    .map(|e| e.code()),
                Some(ErrorCode::InvalidHandle),
                "{transport:?}: terminal connection failure retained the endpoint"
            );
            let failures: Vec<_> = events
                .iter()
                .filter(|e| e.event_type == EventType::JoinFailed)
                .collect();
            let closes: Vec<_> = events
                .iter()
                .filter(|e| e.event_type == EventType::SessionClosed)
                .collect();
            assert_eq!(failures.len(), 1, "{events:?}");
            assert_eq!(closes.len(), 1, "{events:?}");
            assert_eq!(failures[0].status, ErrorCode::Timeout);
            assert_eq!(failures[0].session, closes[0].session);
            assert!(
                events
                    .iter()
                    .position(|e| e.event_type == EventType::JoinFailed)
                    < events
                        .iter()
                        .position(|e| e.event_type == EventType::SessionClosed)
            );
            let metrics = runtime.metrics_snapshot();
            assert_eq!(metrics.current_endpoints, 0);
            assert_eq!(metrics.current_sessions, 0);
            assert_eq!(metrics.pending_handshakes, 0);
        }
        assert_eq!(
            runtime
                .metrics_snapshot()
                .closed_sessions(ErrorCode::Timeout),
            3
        );
    }
}

#[test]
fn total_deadline_reclaims_endpoint_and_allows_repeated_connections() {
    failed_attempts_release_capacity(false);
}

#[test]
fn last_candidate_timeout_reclaims_endpoint_and_reports_join_failure() {
    failed_attempts_release_capacity(true);
}
