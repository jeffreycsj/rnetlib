use super::flush_deferred;
use crate::adaptive_peer::Peer;
use crate::adaptive_wire::DatagramWire;
use crate::state::{session_active, Outbound, SessionTarget};
use crate::{NetworkRuntime, RuntimeConfig};
use bytes::Bytes;
use rnet_core::{ErrorCode, EventType, Transport};
use rnet_protocol::control::SecurityMode;
use rnet_security::transition::{Role, SecurityController};
use rnet_security::{DatagramTransport, InitiatorHandshake, Keypair, ResponderHandshake};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

fn transport() -> DatagramTransport {
    let client_key = Keypair::generate().unwrap();
    let server_key = Keypair::generate().unwrap();
    let mut client = InitiatorHandshake::new(&client_key.private, &server_key.public).unwrap();
    let mut server = ResponderHandshake::new(&server_key.private).unwrap();
    server.read_first(&client.write_first().unwrap()).unwrap();
    client
        .read_response(&server.write_response().unwrap())
        .unwrap();
    let (finish, transport) = client.finish_datagram(&[]).unwrap();
    server.finish_datagram(&finish).unwrap();
    transport
}

#[derive(Clone, Copy)]
enum Fault {
    Encode,
    Socket,
    Backpressure,
}

fn exercise(fault: Fault) {
    let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
    runtime.poll_events(8, Duration::ZERO);
    runtime.runtime.block_on(async {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        // An IPv4-only socket cannot send to IPv6; no external network or timing race is needed.
        let peer: SocketAddr = "[::1]:9".parse().unwrap();
        let (sender, _receiver) = mpsc::channel(8);
        let (cleanup, mut cleanup_rx) = mpsc::unbounded_channel();
        let target = if matches!(fault, Fault::Backpressure) {
            SessionTarget::Kcp {
                sender,
                peer,
                cleanup: Some(cleanup),
            }
        } else {
            SessionTarget::Udp {
                sender,
                peer,
                max_frame_len: 1200,
                cleanup: Some(cleanup),
            }
        };
        let session = runtime.insert_session(77, target);
        let mut peers = HashMap::from([(
            peer,
            Peer::Established {
                session,
                transport: transport(),
                controller: SecurityController::new(Role::Client, SecurityMode::Encrypted, 1),
                commands: None,
                last_activity: Instant::now(),
                transition_deadline: None,
            },
        )]);
        let bytes = if matches!(fault, Fault::Encode) {
            Bytes::from(vec![0; runtime.shared.config.max_body_len + 1])
        } else {
            Bytes::from_static(b"private-queued-payload")
        };
        let kind = if matches!(fault, Fault::Backpressure) {
            Transport::Kcp
        } else {
            Transport::Udp
        };
        let mut wire = DatagramWire::new_with_limits(kind, 1200, 8, 4096, 1);
        wire.authorize_peer(peer, 37).unwrap();
        let mut deferred = HashMap::from([(
            peer,
            VecDeque::from([
                Outbound::new(bytes.clone()),
                Outbound::new(Bytes::from_static(b"second message")),
            ]),
        )]);
        let mut count = 2;
        flush_deferred(
            &runtime.shared,
            &socket,
            &mut wire,
            &mut peers,
            &mut deferred,
            &mut count,
            &mut Vec::new(),
            &mut HashMap::new(),
        )
        .await;
        let events = runtime.poll_events(8, Duration::ZERO);
        if matches!(fault, Fault::Backpressure) {
            assert!(events.is_empty(), "backpressure must not close a session");
            assert!(session_active(&runtime.shared, session));
            assert_eq!(count, 2);
            assert_eq!(deferred[&peer][0].bytes, bytes);
            assert!(peers.contains_key(&peer));
            assert!(wire.has_peer(peer));
            assert!(cleanup_rx.try_recv().is_err());
            assert_eq!(runtime.metrics_snapshot().protocol_errors, 0);
        } else {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_type, EventType::SessionClosed);
            assert_eq!(events[0].endpoint, 77);
            assert_eq!(events[0].session, session);
            let detail = std::str::from_utf8(&events[0].data).unwrap();
            let (code, phase, cause) = if matches!(fault, Fault::Encode) {
                (
                    ErrorCode::MessageTooLarge,
                    "datagram_deferred_encode",
                    "configured limit",
                )
            } else {
                (ErrorCode::IoError, "datagram_deferred_write", "os error")
            };
            assert_eq!(events[0].status, code);
            assert!(detail.contains(phase), "{detail}");
            assert!(detail.contains(cause), "{detail}");
            assert!(!detail.contains("private-queued-payload"));
            assert!(!session_active(&runtime.shared, session));
            assert_eq!(count, 0);
            assert!(deferred.is_empty());
            assert!(peers.is_empty());
            assert!(!wire.has_peer(peer));
            assert_eq!(cleanup_rx.try_recv().unwrap().session, session);
            assert!(cleanup_rx.try_recv().is_err());
            assert_eq!(runtime.metrics_snapshot().closed_sessions(code), 1);
        }
    });
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn deferred_encode_failure_retains_cause_and_cleans_pending_messages() {
    exercise(Fault::Encode);
}

#[test]
fn deferred_socket_failure_retains_os_error_and_cleans_pending_messages() {
    exercise(Fault::Socket);
}

#[test]
fn kcp_backpressure_preserves_deferred_message_without_closing_or_logging() {
    exercise(Fault::Backpressure);
}
