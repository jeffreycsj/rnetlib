//! Exercise local early-data exhaustion through real transport events, including route closure.

use crate::envelope::{encode_application, encode_udp_application};
use crate::range_state::RangeRuntimeState;
use crate::{
    GameEvent, GameProtocolRange, GameRangeClientConfig, GameRangeServerConfig, GameRuntime,
    GameRuntimeConfig,
};
use rnet_core::{ErrorCode, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::{Duration, Instant};

#[test]
fn early_data_exhaustion_closes_real_sessions_without_counting_malformed_plaintext() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        for encrypted in [false, true] {
            let server_key = Keypair::generate().unwrap();
            let mut config = GameRuntimeConfig::default().allow_plaintext_business_data(true);
            config.network.worker_threads = 1;
            let server = GameRuntime::new(config).unwrap();
            let mut config = GameRuntimeConfig::default();
            config.network.worker_threads = 1;
            let client = GameRuntime::new_with_client_security(
                config,
                ClientSecurity::pinned(Keypair::generate().unwrap(), server_key.public.clone()),
            )
            .unwrap();
            let protocol = GameProtocolRange::new(96, 1, 2);
            let listener = server
                .listen_range(GameRangeServerConfig {
                    transport,
                    bind_addr: "127.0.0.1:0".parse().unwrap(),
                    local_key: server_key,
                    initial_encryption: encrypted,
                    protocol,
                })
                .unwrap();
            client
                .connect_range(GameRangeClientConfig {
                    transport,
                    bind_addr: None,
                    remote_addr: server.endpoint_local_addr(listener).unwrap(),
                    join_ticket: b"ticket".to_vec(),
                    protocol,
                })
                .unwrap();
            // Keep the transport queue large, but make the independent game early-data budget
            // one slot. The second empty message must fail admission, not transport enqueueing.
            // Preserve the pending endpoint policy installed by connect_range.
            {
                let mut range = client.range.lock().unwrap();
                assert!(range.sessions.is_empty());
                let pending = std::mem::take(&mut range.client_endpoints);
                *range = RangeRuntimeState::with_buffer_limits(1, 1024);
                range.client_endpoints = pending;
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            let server_session = loop {
                assert!(
                    Instant::now() < deadline,
                    "authorization timeout: {transport:?}"
                );
                let events = server.poll(1, Duration::from_millis(5));
                if let Some(GameEvent::AuthRequest { session, .. }) = events.first() {
                    server.auth_decide(*session, true).unwrap();
                    break *session;
                }
            };
            // Do not game-poll the server after authorization: its SELECT remains unsent.
            // Drive the client's actual transport-open event into the negotiation state.
            let client_session = loop {
                assert!(Instant::now() < deadline, "open timeout: {transport:?}");
                let events = client.network.poll_events(1, Duration::from_millis(5));
                if let Some(event) = events.into_iter().next() {
                    let opened = event.event_type == EventType::SessionOpened;
                    let session = event.session;
                    assert!(!matches!(
                        event.event_type,
                        EventType::SessionClosed | EventType::JoinFailed
                    ));
                    client.convert_event(event).unwrap();
                    if opened {
                        break session;
                    }
                }
            };
            loop {
                assert!(
                    Instant::now() < deadline,
                    "server open timeout: {transport:?}"
                );
                let events = server.network.poll_events(1, Duration::from_millis(5));
                if let Some(event) = events.into_iter().next() {
                    assert!(!matches!(
                        event.event_type,
                        EventType::SessionClosed | EventType::JoinFailed
                    ));
                    if event.event_type == EventType::SessionOpened {
                        assert_eq!(event.session, server_session);
                        break;
                    }
                }
            }
            for sequence in 0..2 {
                let envelope = if transport == Transport::Udp {
                    encode_udp_application(b"", None, None, sequence, 1024).unwrap()
                } else {
                    encode_application(b"", None, None, 1024).unwrap()
                };
                server.network.send(server_session, &envelope).unwrap();
            }
            let mut violation = false;
            let mut closed = false;
            while !violation || !closed {
                assert!(
                    Instant::now() < deadline,
                    "budget failure not surfaced: {transport:?}, encrypted={encrypted}"
                );
                for event in client.poll(16, Duration::from_millis(5)) {
                    match event {
                        GameEvent::ProtocolViolation { session, .. } => {
                            assert_eq!(session, client_session);
                            violation = true;
                        }
                        GameEvent::SessionClosed {
                            session, reason, ..
                        } => {
                            assert_eq!(session, client_session);
                            assert_eq!(reason, ErrorCode::ProtocolError);
                            closed = true;
                        }
                        GameEvent::Message(_) | GameEvent::SessionReady { .. } => {
                            panic!("pre-ready data escaped: {event:?}")
                        }
                        _ => {}
                    }
                }
            }
            assert_eq!(
                client
                    .network
                    .validate_payload_len(client_session, 0)
                    .unwrap_err()
                    .code(),
                ErrorCode::InvalidHandle
            );
            let metrics = client.range_buffer_snapshot();
            assert_eq!(metrics.runtime_admission_rejected, 1);
            assert_eq!(metrics.session_admission_rejected, 0);
            assert_eq!(
                metrics.buffered_messages, 0,
                "close must release the first reservation"
            );
            assert_eq!(
                client
                    .protocol_metrics_snapshot()
                    .plaintext_invalid_envelopes_dropped,
                0
            );
        }
    }
}
