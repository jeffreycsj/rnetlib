//! Driver regressions: hold a real security barrier open while another command is admitted.

use crate::adaptive_tcp::{decrypt_protected, read_wire, write_protected};
use crate::state::{SecurityCommand, SessionTarget};
use crate::{NetworkRuntime, RuntimeConfig, SecurityChange, SecurityMode};
use rnet_core::{ErrorCode, EventType};
use rnet_protocol::control::{
    decode_control, encode_control, Control, ControlKind, ProtectedKind, Record, RecordKind,
};
use rnet_security::transition::{Effect, Role, SecurityController};
use rnet_security::{InitiatorHandshake, Keypair, ResponderHandshake, SecureTransport};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

fn handshakes() -> (InitiatorHandshake, ResponderHandshake) {
    let client_key = Keypair::generate().unwrap();
    let server_key = Keypair::generate().unwrap();
    let mut client = InitiatorHandshake::new(&client_key.private, &server_key.public).unwrap();
    let mut server = ResponderHandshake::new(&server_key.private).unwrap();
    server.read_first(&client.write_first().unwrap()).unwrap();
    client
        .read_response(&server.write_response().unwrap())
        .unwrap();
    (client, server)
}

async fn read(stream: &mut TcpStream) -> Record {
    tokio::time::timeout(Duration::from_secs(2), read_wire(stream, 4096))
        .await
        .expect("security response timed out")
        .unwrap()
}

async fn reply(
    stream: &mut TcpStream,
    transport: &mut SecureTransport,
    controller: &mut SecurityController,
    record: Record,
) {
    let message = decrypt_protected(transport, &record, 4096).unwrap();
    assert_eq!(message.kind, ProtectedKind::Control);
    let transition = controller
        .handle(decode_control(&message.payload).unwrap())
        .unwrap();
    if transition.before_response == Effect::Rekey {
        transport.rekey_incoming();
        transport.rekey_outgoing();
    }
    write_protected(
        stream,
        transport,
        record.epoch,
        ProtectedKind::Control,
        &encode_control(transition.response.unwrap()),
        64,
    )
    .await
    .unwrap();
    if transition.after_response == Effect::Rekey {
        transport.rekey_incoming();
        transport.rekey_outgoing();
    }
}

#[derive(Clone, Copy, Debug)]
enum First {
    Rekey,
    Switch,
    Automatic,
}

fn tcp_queued_command(first: First, complete: bool) {
    let mut config = RuntimeConfig {
        worker_threads: 1,
        ..RuntimeConfig::production()
    };
    config.security_policy.allow_plaintext_business_data = true;
    config.security_policy.rekey_after = None;
    config.security_policy.rekey_after_bytes = matches!(first, First::Automatic).then_some(1);
    if !complete {
        config.handshake_timeout = Duration::from_secs(1);
    }
    let runtime = NetworkRuntime::new(config).unwrap();
    runtime.runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        peer.set_nodelay(true).unwrap();
        let (server_socket, _) = listener.accept().await.unwrap();
        server_socket.set_nodelay(true).unwrap();
        let (client, server) = handshakes();
        let (finish, mut peer_crypto) = client.finish(&[]).unwrap();
        let (_, _, server_crypto) = server.finish(&finish).unwrap();
        let (out_tx, out_rx) = mpsc::channel(4);
        let (commands, commands_rx) = mpsc::channel(1);
        let session = runtime.insert_session(77, SessionTarget::Tcp(out_tx));
        runtime
            .shared
            .sessions
            .lock()
            .unwrap()
            .get_mut(session)
            .unwrap()
            .security_commands = Some(commands.clone());
        let task = runtime
            .runtime
            .spawn(crate::adaptive_tcp_session::run_adaptive_session(
                Arc::clone(&runtime.shared),
                77,
                session,
                server_socket,
                out_rx,
                Some(commands_rx),
                server_crypto,
                SecurityController::new(Role::Server, SecurityMode::Encrypted, 1),
            ));
        match first {
            First::Rekey => runtime.rekey_session(session).unwrap(),
            First::Switch => runtime
                .set_security_mode(session, SecurityMode::Plaintext)
                .unwrap(),
            First::Automatic => {
                runtime.send(session, b"trigger auto rekey").unwrap();
                let record = read(&mut peer).await;
                assert_eq!(
                    decrypt_protected(&mut peer_crypto, &record, 4096)
                        .unwrap()
                        .kind,
                    ProtectedKind::Data
                );
            }
        }
        let proposal = read(&mut peer).await;
        // Seeing the proposal proves the first command was consumed. Deliberately withhold
        // its authenticated reply: a queued second command must not disappear in this interval.
        if matches!(first, First::Switch) {
            runtime.rekey_session(session).unwrap();
        } else {
            runtime
                .set_security_mode(session, SecurityMode::Plaintext)
                .unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), commands.reserve())
                .await
                .is_err(),
            "driver consumed the next command before the current barrier completed"
        );
        assert_eq!(
            runtime.rekey_session(session).unwrap_err().code(),
            ErrorCode::WouldBlock
        );

        if !complete {
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("queued command must not suppress the barrier deadline")
                .unwrap();
            assert!(commands.is_closed());
            assert!(!crate::state::session_active(&runtime.shared, session));
            let events = runtime.poll_events(16, Duration::ZERO);
            let closed: Vec<_> = events
                .iter()
                .filter(|event| event.event_type == EventType::SessionClosed)
                .collect();
            assert_eq!(closed.len(), 1);
            assert_eq!(closed[0].session, session);
            assert_eq!(closed[0].status, ErrorCode::Timeout);
            assert!(!events
                .iter()
                .any(|event| event.event_type == EventType::SecurityChanged));
            return;
        }

        let mut controller = SecurityController::new(Role::Client, SecurityMode::Encrypted, 1);
        reply(&mut peer, &mut peer_crypto, &mut controller, proposal).await;
        let commit = read(&mut peer).await;
        reply(&mut peer, &mut peer_crypto, &mut controller, commit).await;
        let next = read(&mut peer).await;
        reply(&mut peer, &mut peer_crypto, &mut controller, next).await;
        let commit = read(&mut peer).await;
        reply(&mut peer, &mut peer_crypto, &mut controller, commit).await;
        assert_eq!(controller.epoch(), 3);
        assert_eq!(controller.mode(), SecurityMode::Plaintext);
        runtime.send(session, b"after both barriers").unwrap();
        let data = read(&mut peer).await;
        assert_eq!(data.kind, RecordKind::PlainData);
        assert_eq!(
            rnet_protocol::decode_datagram(&data.payload, 4096)
                .unwrap()
                .body
                .as_ref(),
            b"after both barriers"
        );
        let epochs: Vec<_> = runtime
            .poll_events(16, Duration::ZERO)
            .iter()
            .filter(|event| event.event_type == EventType::SecurityChanged)
            .map(|event| SecurityChange::from_event(event).unwrap().epoch)
            .collect();
        assert_eq!(
            epochs,
            [2, 3],
            "each local barrier must publish exactly once in order"
        );
        runtime
            .close_session(session, ErrorCode::Cancelled)
            .unwrap();
        task.abort();
        let _ = task.await;
    });
}

#[test]
fn tcp_retains_mode_request_during_rekey() {
    tcp_queued_command(First::Rekey, true);
}

#[test]
fn tcp_retains_rekey_request_during_mode_switch() {
    tcp_queued_command(First::Switch, true);
}

#[test]
fn tcp_retains_mode_request_during_automatic_rekey() {
    tcp_queued_command(First::Automatic, true);
}

#[test]
fn tcp_barrier_timeout_cancels_pending_commands_without_success_events() {
    tcp_queued_command(First::Rekey, false);
}

#[test]
fn datagram_retains_commands_during_manual_and_automatic_barriers() {
    use crate::adaptive_datagram_maintenance::drive_server;
    use crate::adaptive_peer::Peer;
    use crate::adaptive_wire::DatagramWire;
    use crate::auto_rekey::AutoRekey;
    use rnet_core::Transport;
    use std::collections::HashMap;
    use std::time::Instant;
    use tokio::net::UdpSocket;

    for kind in [Transport::Udp, Transport::Kcp] {
        for first in [First::Rekey, First::Switch, First::Automatic] {
            let mut config = RuntimeConfig {
                worker_threads: 1,
                ..RuntimeConfig::production()
            };
            config.security_policy.allow_plaintext_business_data = true;
            let runtime = NetworkRuntime::new(config).unwrap();
            runtime.runtime.block_on(async {
                let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
                let peer_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let peer = peer_socket.local_addr().unwrap();
                let (client, server) = handshakes();
                let (finish, _) = client.finish_datagram(&[]).unwrap();
                let (_, _, crypto) = server.finish_datagram(&finish).unwrap();
                let (tx, rx) = mpsc::channel(1);
                let mut peers = HashMap::from([(
                    peer,
                    Peer::Established {
                        session: 1,
                        transport: crypto,
                        controller: SecurityController::new(
                            Role::Server,
                            SecurityMode::Encrypted,
                            1,
                        ),
                        commands: Some(rx),
                        last_activity: Instant::now(),
                        transition_deadline: None,
                    },
                )]);
                let mut wire = DatagramWire::new(kind, 1200, 4);
                wire.authorize_peer(peer, 17).unwrap();
                let mut timer = AutoRekey::new(None, Some(1), Instant::now());
                match first {
                    First::Automatic => timer.record_encrypted_bytes(1),
                    First::Rekey => tx.try_send(SecurityCommand::Rekey).unwrap(),
                    First::Switch => tx
                        .try_send(SecurityCommand::SetMode(SecurityMode::Plaintext))
                        .unwrap(),
                }
                let mut timers = HashMap::from([(peer, timer)]);
                let mut scratch = Vec::new();
                drive_server(
                    &runtime.shared,
                    77,
                    &socket,
                    &mut wire,
                    &mut peers,
                    &mut scratch,
                    &mut timers,
                )
                .await;
                let next = if matches!(first, First::Switch) {
                    SecurityCommand::Rekey
                } else {
                    SecurityCommand::SetMode(SecurityMode::Plaintext)
                };
                tx.try_send(next).unwrap();
                for _ in 0..3 {
                    drive_server(
                        &runtime.shared,
                        77,
                        &socket,
                        &mut wire,
                        &mut peers,
                        &mut scratch,
                        &mut timers,
                    )
                    .await;
                    assert_eq!(
                        tx.capacity(),
                        0,
                        "{kind:?}, first={first:?}: queued command lost"
                    );
                }
                // Complete the first controller barrier deterministically. Existing loopback
                // tests cover the datagram cipher/wire exchange; here the driver queue is at issue.
                let Peer::Established {
                    controller,
                    transport,
                    transition_deadline,
                    ..
                } = peers.get_mut(&peer).unwrap()
                else {
                    panic!("peer disappeared")
                };
                let rekey = [ControlKind::RekeyReady, ControlKind::RekeyAck];
                let switch = [ControlKind::SwitchReady, ControlKind::SwitchAck];
                let (initial, next) = if matches!(first, First::Switch) {
                    (switch, rekey)
                } else {
                    (rekey, switch)
                };
                for op in initial {
                    let step = controller.handle(Control::barrier(op, 2)).unwrap();
                    crate::adaptive_codec::apply(transport, step.before_response);
                    crate::adaptive_codec::apply(transport, step.after_response);
                }
                *transition_deadline = None;
                drive_server(
                    &runtime.shared,
                    77,
                    &socket,
                    &mut wire,
                    &mut peers,
                    &mut scratch,
                    &mut timers,
                )
                .await;
                assert_eq!(tx.capacity(), 1, "next command was never executed");
                let Peer::Established { controller, .. } = peers.get_mut(&peer).unwrap() else {
                    panic!("peer disappeared")
                };
                for op in next {
                    controller.handle(Control::barrier(op, 3)).unwrap();
                }
                assert_eq!(controller.mode(), SecurityMode::Plaintext);
                assert_eq!(controller.epoch(), 3);
            });
        }
    }
}
