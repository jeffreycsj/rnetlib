//! Timer-driven authentication, security-transition, and peer cleanup work.

use crate::adaptive_datagram_session::send_protected;
use crate::adaptive_peer::{Peer, INITIAL_EPOCH};
use crate::adaptive_wire::DatagramWire;
use crate::auto_rekey::AutoRekey;
use crate::state::{
    mark_session_established, remove_session_with_reason, session_event, try_push_session_event,
    Outbound, SecurityCommand, Shared,
};
use rnet_core::{ErrorCode, EventType, Handle};
use rnet_protocol::control::{encode_control, ProtectedKind};
use rnet_security::transition::{Role, SecurityController};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;

pub(crate) fn cleanup_peer(
    peers: &mut HashMap<SocketAddr, Peer>,
    deferred: &mut HashMap<SocketAddr, VecDeque<Outbound>>,
    deferred_count: &mut usize,
    wire: &mut DatagramWire,
    peer: SocketAddr,
    session: Handle,
) {
    if let Some(current) = peers.get(&peer) {
        if matches!(current, Peer::ServerReject { .. }) {
            // The session is gone, but UDP/KCP still owns a bounded terminal control response.
            // Dropping it here would turn an explicit rejection into a client-side timeout.
            return;
        }
        if current.session() != Some(session) {
            return;
        }
    }
    peers.remove(&peer);
    if let Some(queued) = deferred.remove(&peer) {
        *deferred_count = deferred_count.saturating_sub(queued.len());
    }
    wire.remove_peer(peer);
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn drive_server(
    shared: &Arc<Shared>,
    endpoint: Handle,
    socket: &Arc<UdpSocket>,
    wire: &mut DatagramWire,
    peers: &mut HashMap<SocketAddr, Peer>,
    scratch_peers: &mut Vec<SocketAddr>,
    automatic_rekeys: &mut HashMap<SocketAddr, AutoRekey>,
    allow_client_timeout: bool,
) {
    scratch_peers.clear();
    scratch_peers.extend(peers.keys().copied());
    for &peer in scratch_peers.iter() {
        let Some(state) = peers.remove(&peer) else {
            continue;
        };
        let state = match state {
            Peer::Cookie { expires_at } if Instant::now() >= expires_at => {
                wire.remove_peer(peer);
                continue;
            }
            Peer::ServerReject { expires_at } if Instant::now() >= expires_at => {
                wire.remove_peer(peer);
                continue;
            }
            Peer::ServerFirst { started, .. } | Peer::ServerFinish { started, .. }
                if started.elapsed() >= shared.config.handshake_timeout =>
            {
                wire.remove_peer(peer);
                continue;
            }
            Peer::ClientHello { session, started }
            | Peer::ClientResponse {
                session, started, ..
            } if allow_client_timeout && started.elapsed() >= shared.config.handshake_timeout => {
                remove_session_with_reason(shared, endpoint, session, ErrorCode::Timeout);
                wire.remove_peer(peer);
                continue;
            }
            Peer::ClientAuth {
                session,
                auth_started,
                ..
            } if allow_client_timeout
                && auth_started.elapsed() >= shared.config.handshake_timeout =>
            {
                remove_session_with_reason(shared, endpoint, session, ErrorCode::Timeout);
                wire.remove_peer(peer);
                continue;
            }
            Peer::ServerAuth {
                session,
                auth_started,
                ..
            } if auth_started.elapsed() >= shared.config.handshake_timeout => {
                remove_session_with_reason(shared, endpoint, session, ErrorCode::Timeout);
                wire.remove_peer(peer);
                continue;
            }
            Peer::ServerAuth {
                session,
                mut transport,
                mut auth,
                commands,
                mode,
                auth_started,
            } => match auth.try_recv() {
                Ok(accepted) => {
                    shared.latencies.record(
                        crate::metrics::LatencyKind::AuthWait,
                        auth_started.elapsed(),
                    );
                    let sent = send_protected(
                        socket,
                        wire,
                        peer,
                        &mut transport,
                        INITIAL_EPOCH,
                        ProtectedKind::AuthDecision,
                        &[u8::from(accepted), mode as u8],
                        64,
                    )
                    .await;
                    if let Err(error) = sent {
                        remove_session_with_reason(shared, endpoint, session, error.code());
                        continue;
                    }
                    if accepted && mark_session_established(shared, session).is_ok() {
                        if try_push_session_event(
                            shared,
                            session_event(EventType::SessionOpened, endpoint, session),
                        )
                        .is_err()
                        {
                            continue;
                        }
                        automatic_rekeys.entry(peer).or_insert_with(|| {
                            AutoRekey::new(
                                shared.config.security_policy.rekey_after,
                                shared.config.security_policy.rekey_after_bytes,
                                Instant::now(),
                            )
                        });
                        Peer::Established {
                            session,
                            transport,
                            controller: SecurityController::new(Role::Server, mode, INITIAL_EPOCH),
                            commands: Some(commands),
                            last_activity: Instant::now(),
                            transition_deadline: None,
                        }
                    } else {
                        remove_session_with_reason(
                            shared,
                            endpoint,
                            session,
                            ErrorCode::AuthRejected,
                        );
                        Peer::ServerReject {
                            expires_at: Instant::now() + shared.config.handshake_timeout,
                        }
                    }
                }
                _ => Peer::ServerAuth {
                    session,
                    transport,
                    auth,
                    commands,
                    mode,
                    auth_started,
                },
            },
            Peer::Established {
                session,
                last_activity,
                ..
            } if last_activity.elapsed() >= shared.config.datagram_idle_timeout => {
                remove_session_with_reason(shared, endpoint, session, ErrorCode::Timeout);
                wire.remove_peer(peer);
                continue;
            }
            Peer::Established {
                session,
                transition_deadline: Some(deadline),
                ..
            } if Instant::now() >= deadline => {
                remove_session_with_reason(shared, endpoint, session, ErrorCode::Timeout);
                wire.remove_peer(peer);
                continue;
            }
            Peer::Established {
                session,
                mut transport,
                mut controller,
                mut commands,
                last_activity,
                mut transition_deadline,
            } => {
                let mut pending_control = None;
                if let Some(receiver) = commands.as_mut() {
                    if let Ok(command) = receiver.try_recv() {
                        let control = match command {
                            SecurityCommand::SetMode(mode) => controller.begin_switch(mode),
                            SecurityCommand::Rekey => {
                                if let Some(rekey) = automatic_rekeys.get_mut(&peer) {
                                    rekey.mark_started(Instant::now());
                                }
                                controller.begin_rekey()
                            }
                        };
                        pending_control = control.ok();
                    }
                }
                if pending_control.is_none()
                    && commands.is_some()
                    && controller.mode() == rnet_protocol::control::SecurityMode::Encrypted
                    && !controller.is_transitioning()
                    && automatic_rekeys
                        .get(&peer)
                        .is_some_and(|rekey| rekey.is_due(Instant::now()))
                {
                    pending_control = controller.begin_rekey().ok();
                    if pending_control.is_some() {
                        if let Some(rekey) = automatic_rekeys.get_mut(&peer) {
                            rekey.mark_started(Instant::now());
                        }
                    }
                }
                if let Some(control) = pending_control {
                    let sent = send_protected(
                        socket,
                        wire,
                        peer,
                        &mut transport,
                        controller.epoch(),
                        ProtectedKind::Control,
                        &encode_control(control),
                        64,
                    )
                    .await;
                    if let Err(error) = sent {
                        remove_session_with_reason(shared, endpoint, session, error.code());
                        continue;
                    }
                    transition_deadline = Some(Instant::now() + shared.config.handshake_timeout);
                }
                Peer::Established {
                    session,
                    transport,
                    controller,
                    commands,
                    last_activity,
                    transition_deadline,
                }
            }
            other => other,
        };
        peers.insert(peer, state);
    }
}

#[cfg(test)]
mod tests {
    use super::cleanup_peer;
    use crate::adaptive_peer::Peer;
    use crate::adaptive_wire::DatagramWire;
    use crate::state::Outbound;
    use bytes::Bytes;
    use rnet_core::Transport;
    use std::collections::{HashMap, VecDeque};
    use std::net::SocketAddr;
    use std::time::Instant;

    #[test]
    fn cleanup_removes_only_the_matching_session_and_its_deferred_messages() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 7010));
        let session = 42;
        let mut peers = HashMap::from([(
            peer,
            Peer::ClientHello {
                session,
                started: Instant::now(),
            },
        )]);
        let mut deferred = HashMap::from([(
            peer,
            VecDeque::from([Outbound::new(Bytes::from_static(b"queued"))]),
        )]);
        let mut deferred_count = 1;
        let mut wire = DatagramWire::new(Transport::Kcp, 1200, 1024);

        cleanup_peer(
            &mut peers,
            &mut deferred,
            &mut deferred_count,
            &mut wire,
            peer,
            session,
        );

        assert!(peers.is_empty());
        assert!(deferred.is_empty());
        assert_eq!(deferred_count, 0);
    }

    #[test]
    fn cleanup_does_not_drop_a_rejection_waiting_for_control_retransmission() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 7011));
        let mut peers = HashMap::from([(
            peer,
            Peer::ServerReject {
                expires_at: Instant::now() + std::time::Duration::from_secs(1),
            },
        )]);
        let mut deferred = HashMap::new();
        let mut deferred_count = 0;
        let mut wire = DatagramWire::new(Transport::Kcp, 1200, 1024);
        cleanup_peer(
            &mut peers,
            &mut deferred,
            &mut deferred_count,
            &mut wire,
            peer,
            42,
        );
        assert!(matches!(peers.get(&peer), Some(Peer::ServerReject { .. })));
    }
}
