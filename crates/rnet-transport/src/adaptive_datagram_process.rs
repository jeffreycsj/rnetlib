//! Adaptive UDP/KCP handshake and established-session state machine.

use crate::adaptive_codec::{decrypt, protocol};
use crate::adaptive_datagram_session::{handle_established, send_protected_response};
use crate::adaptive_peer::{Peer, INITIAL_EPOCH};
use crate::adaptive_wire::DatagramWire;
use crate::config::{ClientSecurity, EndpointSecurity};
use crate::cookie::{CookieGuard, COOKIE_LEN};
use crate::state::{
    insert_session_route, map_security_error, mark_session_established, session_event,
    try_push_session_event, ByteBudget, DatagramCleanup, Outbound, SessionRoute, SessionTarget,
    Shared,
};
use rnet_core::{ErrorCode, EventType, Handle, Result, RnetError, Transport};
use rnet_protocol::control::{ProtectedKind, Record, RecordKind, SecurityMode};
use rnet_security::transition::{Role, SecurityController};
use rnet_security::{InitiatorHandshake, ResponderHandshake};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn process(
    shared: &Arc<Shared>,
    endpoint: Handle,
    socket: &Arc<UdpSocket>,
    wire: &mut DatagramWire,
    sender: &mpsc::Sender<(SocketAddr, Outbound)>,
    security: &EndpointSecurity,
    client_security: Option<&ClientSecurity>,
    cleanup_sender: &mpsc::UnboundedSender<DatagramCleanup>,
    transport_kind: Transport,
    cookie: &CookieGuard,
    peers: &mut HashMap<SocketAddr, Peer>,
    peer: SocketAddr,
    record: Record,
) -> Result<()> {
    let previous = peers.remove(&peer);
    let next = match (security, previous, record.kind) {
        (EndpointSecurity::AdaptiveServer { .. }, Some(state @ Peer::ServerReject { .. }), _) => {
            Some(state)
        }
        (EndpointSecurity::AdaptiveServer { .. }, previous, RecordKind::ClientHello)
            if record.payload.len() != COOKIE_LEN || !cookie.validate(peer, &record.payload) =>
        {
            wire.send_response(
                socket,
                peer,
                &Record::new(RecordKind::CookieChallenge, 0, &cookie.issue(peer)),
                128,
            )
            .await?;
            if transport_kind == Transport::Kcp {
                Some(previous.unwrap_or(Peer::Cookie {
                    expires_at: Instant::now() + shared.config.handshake_timeout,
                }))
            } else {
                previous
            }
        }
        (EndpointSecurity::AdaptiveServer { .. }, Some(state), RecordKind::ClientHello)
            if state.session().is_some() =>
        {
            Some(state)
        }
        (
            EndpointSecurity::AdaptiveServer {
                local_key,
                initial_mode,
            },
            _,
            RecordKind::ClientHello,
        ) => {
            if peers.len() >= shared.config.max_sessions_per_endpoint && !peers.contains_key(&peer)
            {
                return Err(RnetError::new(
                    ErrorCode::RateLimited,
                    "adaptive datagram peer limit reached",
                ));
            }
            let mut hello = vec![*initial_mode as u8];
            hello.extend_from_slice(&local_key.public);
            wire.send_reliable(
                socket,
                peer,
                &Record::new(RecordKind::ServerHello, 0, &hello),
                128,
            )
            .await?;
            Some(Peer::ServerFirst {
                handshake: ResponderHandshake::new(&local_key.private)
                    .map_err(map_security_error)?,
                started: Instant::now(),
            })
        }
        (
            EndpointSecurity::AdaptiveClient { .. },
            Some(Peer::ClientHello { session, started }),
            RecordKind::CookieChallenge,
        ) => {
            wire.send_reliable(
                socket,
                peer,
                &Record::new(RecordKind::ClientHello, 0, &record.payload),
                128,
            )
            .await?;
            Some(Peer::ClientHello { session, started })
        }
        (
            EndpointSecurity::AdaptiveClient { join_payload },
            Some(Peer::ClientHello { session, started }),
            RecordKind::ServerHello,
        ) => {
            if record.payload.len() != 33 {
                return protocol("invalid server hello");
            }
            let mode = SecurityMode::try_from(record.payload[0])?;
            let key: [u8; 32] = record.payload[1..].try_into().expect("key length");
            let identity = client_security.ok_or_else(|| {
                RnetError::new(ErrorCode::InvalidState, "missing client security")
            })?;
            if !(identity.peer_verifier)(&key) {
                return Err(RnetError::new(
                    ErrorCode::PeerKeyMismatch,
                    "server rejected",
                ));
            }
            let mut handshake = InitiatorHandshake::new(&identity.local_key.private, &key)
                .map_err(map_security_error)?;
            let first = handshake.write_first().map_err(map_security_error)?;
            wire.send_reliable(
                socket,
                peer,
                &Record::new(RecordKind::Handshake, 0, &first),
                shared.config.max_datagram_size,
            )
            .await?;
            Some(Peer::ClientResponse {
                session,
                handshake,
                join: join_payload.clone(),
                mode,
                started,
            })
        }
        (
            _,
            Some(Peer::ServerFirst {
                mut handshake,
                started,
            }),
            RecordKind::Handshake,
        ) => {
            handshake
                .read_first(&record.payload)
                .map_err(map_security_error)?;
            let response = handshake.write_response().map_err(map_security_error)?;
            wire.send_reliable(
                socket,
                peer,
                &Record::new(RecordKind::Handshake, 0, &response),
                shared.config.max_datagram_size,
            )
            .await?;
            Some(Peer::ServerFinish { handshake, started })
        }
        (
            _,
            Some(Peer::ClientResponse {
                session,
                mut handshake,
                join,
                mode,
                started,
            }),
            RecordKind::Handshake,
        ) => {
            handshake
                .read_response(&record.payload)
                .map_err(map_security_error)?;
            let (finish, transport) = handshake
                .finish_datagram(&join)
                .map_err(map_security_error)?;
            wire.send_reliable(
                socket,
                peer,
                &Record::new(RecordKind::Handshake, 0, &finish),
                shared.config.max_datagram_size,
            )
            .await?;
            shared.latencies.record(
                crate::metrics::LatencyKind::CryptoHandshake,
                started.elapsed(),
            );
            Some(Peer::ClientAuth {
                session,
                transport,
                mode,
                auth_started: Instant::now(),
                connect_started: started,
            })
        }
        (
            EndpointSecurity::AdaptiveServer { initial_mode, .. },
            Some(Peer::ServerFinish { handshake, started }),
            RecordKind::Handshake,
        ) => {
            let (join, peer_key, transport) = handshake
                .finish_datagram(&record.payload)
                .map_err(map_security_error)?;
            shared.latencies.record(
                crate::metrics::LatencyKind::CryptoHandshake,
                started.elapsed(),
            );
            let (auth_tx, auth) = oneshot::channel();
            let (cmd_tx, commands) = mpsc::channel(1);
            let session = insert_session_route(
                shared,
                SessionRoute {
                    endpoint,
                    target: match transport_kind {
                        Transport::Kcp => SessionTarget::Kcp {
                            sender: sender.clone(),
                            peer,
                            cleanup: Some(cleanup_sender.clone()),
                        },
                        _ => SessionTarget::Udp {
                            sender: sender.clone(),
                            peer,
                            max_frame_len: shared
                                .config
                                .max_datagram_size
                                .saturating_sub(crate::adaptive_codec::ENCRYPTED_UDP_WIRE_OVERHEAD),
                            cleanup: Some(cleanup_sender.clone()),
                        },
                    },
                    established: false,
                    auth_decision: Some(auth_tx),
                    security_commands: Some(cmd_tx),
                    allows_game_controls: true,
                    queued_bytes: ByteBudget::new(shared.config.max_session_queued_bytes),
                },
            )?;
            let mut event = session_event(EventType::AuthRequest, endpoint, session);
            event.data.extend_from_slice(&peer_key);
            event.data.extend_from_slice(&join);
            try_push_session_event(shared, event)?;
            let _ = initial_mode;
            Some(Peer::ServerAuth {
                session,
                transport,
                auth,
                commands,
                mode: *initial_mode,
                auth_started: Instant::now(),
            })
        }
        (
            _,
            Some(Peer::ClientAuth {
                session,
                mut transport,
                mode,
                auth_started,
                connect_started,
            }),
            RecordKind::Protected,
        ) => {
            let message = decrypt(&mut transport, &record, shared.config.max_body_len)?;
            if message.kind != ProtectedKind::AuthDecision {
                return Err(RnetError::new(
                    ErrorCode::AuthRejected,
                    "authentication rejected",
                ));
            }
            match message.payload.as_slice() {
                [1, authenticated_mode] if *authenticated_mode == mode as u8 => {}
                [1, _] => return protocol("server security mode changed during negotiation"),
                _ => {
                    return Err(RnetError::new(
                        ErrorCode::AuthRejected,
                        "authentication rejected",
                    ));
                }
            }
            shared.latencies.record(
                crate::metrics::LatencyKind::AuthWait,
                auth_started.elapsed(),
            );
            shared.latencies.record(
                crate::metrics::LatencyKind::Connect,
                connect_started.elapsed(),
            );
            send_protected_response(
                socket,
                wire,
                peer,
                &mut transport,
                INITIAL_EPOCH,
                ProtectedKind::AuthAck,
                &[],
                32,
            )
            .await?;
            mark_session_established(shared, session)?;
            try_push_session_event(
                shared,
                session_event(EventType::SessionOpened, endpoint, session),
            )?;
            Some(Peer::Established {
                session,
                transport,
                controller: SecurityController::new(Role::Client, mode, INITIAL_EPOCH),
                commands: None,
                last_activity: Instant::now(),
                transition_deadline: None,
            })
        }
        (
            _,
            Some(Peer::Established {
                session,
                mut transport,
                mut controller,
                commands,
                last_activity,
                mut transition_deadline,
            }),
            _,
        ) => {
            let result = handle_established(
                shared,
                endpoint,
                socket,
                wire,
                peer,
                session,
                &mut transport,
                &mut controller,
                record,
            )
            .await;
            if let Err(error) = result {
                peers.insert(
                    peer,
                    Peer::Established {
                        session,
                        transport,
                        controller,
                        commands,
                        last_activity,
                        transition_deadline,
                    },
                );
                return Err(error);
            }
            if !controller.is_transitioning() {
                transition_deadline = None;
            }
            Some(Peer::Established {
                session,
                transport,
                controller,
                commands,
                last_activity: Instant::now(),
                transition_deadline,
            })
        }
        (_, old, _) => old,
    };
    if let Some(state) = next {
        peers.insert(peer, state);
    }
    Ok(())
}
