use crate::{
    config::EndpointSecurity,
    cookie::{CookieGuard, COOKIE_LEN},
    metrics::LatencyKind,
    state::{
        fail_secure_session, insert_session_route, map_security_error, mark_session_established,
        message_event, push_tcp_event, remove_session, session_active, session_event, ByteBudget,
        Outbound, SessionRoute, SessionTarget, Shared,
    },
    HANDSHAKE_TIMEOUT,
};
use rnet_core::{ErrorCode, EventType, Handle, Result, RnetError, Transport};
use rnet_protocol::decode_datagram;
use rnet_security::{DatagramTransport, InitiatorHandshake, ResponderHandshake};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

pub(crate) const UDP_HANDSHAKE_FIRST: u8 = 1;
const UDP_HANDSHAKE_RESPONSE: u8 = 2;
const UDP_HANDSHAKE_FINISH: u8 = 3;
const UDP_AUTH_DECISION: u8 = 4;
pub(crate) const UDP_DATA: u8 = 5;
const UDP_COOKIE_CHALLENGE: u8 = 6;
pub(crate) const MAX_SECURE_DATAGRAM_PEERS: usize = 1024;

pub(crate) enum SecureUdpPeer {
    ClientWaitCookie {
        session: Handle,
        handshake: InitiatorHandshake,
        first: Vec<u8>,
        join_payload: Vec<u8>,
        handshake_started: Instant,
    },
    ServerWaitFinish {
        handshake: ResponderHandshake,
        deadline: Instant,
        handshake_started: Instant,
    },
    ServerAuth {
        session: Handle,
        transport: DatagramTransport,
        decision: oneshot::Receiver<bool>,
        deadline: Instant,
        auth_started: Instant,
    },
    ClientWaitResponse {
        session: Handle,
        handshake: InitiatorHandshake,
        join_payload: Vec<u8>,
        handshake_started: Instant,
    },
    ClientWaitDecision {
        session: Handle,
        transport: DatagramTransport,
        auth_started: Instant,
    },
    Established {
        session: Handle,
        transport: DatagramTransport,
    },
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_secure_udp_endpoint(
    shared: Arc<Shared>,
    endpoint: Handle,
    socket: Arc<UdpSocket>,
    sender: mpsc::Sender<(SocketAddr, Outbound)>,
    mut receiver: mpsc::Receiver<(SocketAddr, Outbound)>,
    remote_addr: Option<SocketAddr>,
    initial_session: Option<Handle>,
    security: EndpointSecurity,
) {
    let mut peers = HashMap::<SocketAddr, SecureUdpPeer>::new();
    let cookie_guard = match CookieGuard::new() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if let (
        Some(peer),
        Some(session),
        EndpointSecurity::Client {
            local_key,
            expected_server_public,
            join_payload,
        },
    ) = (remote_addr, initial_session, &security)
    {
        match InitiatorHandshake::new(&local_key.private, expected_server_public).and_then(
            |mut handshake| {
                let first = handshake.write_first()?;
                Ok((handshake, first))
            },
        ) {
            Ok((handshake, first)) => {
                if send_udp_control(&socket, peer, UDP_HANDSHAKE_FIRST, &first)
                    .await
                    .is_ok()
                {
                    peers.insert(
                        peer,
                        SecureUdpPeer::ClientWaitCookie {
                            session,
                            handshake,
                            first,
                            join_payload: join_payload.clone(),
                            handshake_started: Instant::now(),
                        },
                    );
                }
            }
            Err(error) => {
                fail_secure_session(&shared, endpoint, session, map_security_error(error))
            }
        }
    }

    let mut buffer = vec![0_u8; shared.config.max_datagram_size + 1];
    let mut auth_tick = tokio::time::interval(Duration::from_millis(5));
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buffer) => {
                let (length, peer) = match received {
                    Ok(value) => value,
                    Err(_) => break,
                };
                if length == 0 || length > shared.config.max_datagram_size {
                    shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let mut outgoing = Vec::<(u8, Vec<u8>)>::new();
                let handled = handle_secure_udp_packet(
                    &shared,
                    endpoint,
                    &sender,
                    &security,
                    &cookie_guard,
                    Transport::Udp,
                    &mut peers,
                    peer,
                    buffer[0],
                    &buffer[1..length],
                    &mut |kind, payload| {
                        outgoing.push((kind, payload.to_vec()));
                        Ok(())
                    },
                )
                .await;
                for (kind, payload) in outgoing {
                    let _ = send_udp_control(&socket, peer, kind, &payload).await;
                }
                if handled.is_err() {
                    shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            outbound = receiver.recv() => {
                let Some((peer, outbound)) = outbound else { break; };
                shared.latencies.record(
                    LatencyKind::SendQueue,
                    outbound.queued_at.elapsed(),
                );
                let frame = outbound.bytes;
                let Some(state) = peers.remove(&peer) else { continue; };
                match state {
                    SecureUdpPeer::Established { session, mut transport } => {
                        if !session_active(&shared, session) {
                            continue;
                        }
                        if let Ok(record) = transport.encrypt(&frame) {
                            let _ = send_udp_control(&socket, peer, UDP_DATA, &record).await;
                            shared.metrics.bytes_sent.fetch_add(frame.len() as u64, Ordering::Relaxed);
                        }
                        peers.insert(peer, SecureUdpPeer::Established { session, transport });
                    }
                    other => {
                        peers.insert(peer, other);
                    }
                }
            }
            _ = auth_tick.tick() => {
                let mut outgoing = Vec::<(SocketAddr, u8, Vec<u8>)>::new();
                process_udp_auth_decisions(
                    &shared,
                    endpoint,
                    &mut peers,
                    &mut |peer, kind, payload| {
                        outgoing.push((peer, kind, payload.to_vec()));
                        Ok(())
                    },
                ).await;
                for (peer, kind, payload) in outgoing {
                    let _ = send_udp_control(&socket, peer, kind, &payload).await;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_secure_udp_packet(
    shared: &Arc<Shared>,
    endpoint: Handle,
    sender: &mpsc::Sender<(SocketAddr, Outbound)>,
    security: &EndpointSecurity,
    cookie_guard: &CookieGuard,
    session_transport: Transport,
    peers: &mut HashMap<SocketAddr, SecureUdpPeer>,
    peer: SocketAddr,
    kind: u8,
    payload: &[u8],
    send_control: &mut (dyn FnMut(u8, &[u8]) -> Result<()> + Send),
) -> Result<()> {
    let previous = peers.remove(&peer);
    if previous.is_none()
        && matches!(security, EndpointSecurity::Server { .. })
        && peers.len() >= MAX_SECURE_DATAGRAM_PEERS
    {
        return Err(RnetError::new(
            ErrorCode::RateLimited,
            "secure datagram peer limit reached",
        ));
    }
    match (security, kind, previous) {
        (EndpointSecurity::Server { .. }, UDP_HANDSHAKE_FIRST, previous)
            if payload.len() < COOKIE_LEN
                || !cookie_guard.validate(peer, &payload[..COOKIE_LEN]) =>
        {
            send_control(UDP_COOKIE_CHALLENGE, &cookie_guard.issue(peer))?;
            if let Some(previous) = previous {
                peers.insert(peer, previous);
            }
        }
        (EndpointSecurity::Server { local_key }, UDP_HANDSHAKE_FIRST, _) => {
            let mut handshake =
                ResponderHandshake::new(&local_key.private).map_err(map_security_error)?;
            handshake
                .read_first(&payload[COOKIE_LEN..])
                .map_err(map_security_error)?;
            let response = handshake.write_response().map_err(map_security_error)?;
            send_control(UDP_HANDSHAKE_RESPONSE, &response)?;
            peers.insert(
                peer,
                SecureUdpPeer::ServerWaitFinish {
                    handshake,
                    deadline: Instant::now() + HANDSHAKE_TIMEOUT,
                    handshake_started: Instant::now(),
                },
            );
        }
        (
            EndpointSecurity::Client { .. },
            UDP_COOKIE_CHALLENGE,
            Some(SecureUdpPeer::ClientWaitCookie {
                session,
                handshake,
                first,
                join_payload,
                handshake_started,
            }),
        ) => {
            if payload.len() != COOKIE_LEN {
                peers.insert(
                    peer,
                    SecureUdpPeer::ClientWaitCookie {
                        session,
                        handshake,
                        first,
                        join_payload,
                        handshake_started,
                    },
                );
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "invalid cookie length",
                ));
            }
            let mut first_with_cookie = Vec::with_capacity(COOKIE_LEN + first.len());
            first_with_cookie.extend_from_slice(payload);
            first_with_cookie.extend_from_slice(&first);
            send_control(UDP_HANDSHAKE_FIRST, &first_with_cookie)?;
            peers.insert(
                peer,
                SecureUdpPeer::ClientWaitResponse {
                    session,
                    handshake,
                    join_payload,
                    handshake_started,
                },
            );
        }
        (
            EndpointSecurity::Server { .. },
            UDP_HANDSHAKE_FINISH,
            Some(SecureUdpPeer::ServerWaitFinish {
                handshake,
                deadline,
                handshake_started,
            }),
        ) => {
            if Instant::now() > deadline {
                return Err(RnetError::new(ErrorCode::Timeout, "handshake timed out"));
            }
            let (join_payload, peer_key, transport) = handshake
                .finish_datagram(payload)
                .map_err(map_security_error)?;
            shared
                .latencies
                .record(LatencyKind::CryptoHandshake, handshake_started.elapsed());
            let (decision_sender, decision) = oneshot::channel();
            let session = insert_session_route(
                shared,
                SessionRoute {
                    endpoint,
                    target: match session_transport {
                        Transport::Kcp => SessionTarget::Kcp {
                            sender: sender.clone(),
                            peer,
                            cleanup: None,
                        },
                        _ => SessionTarget::Udp {
                            sender: sender.clone(),
                            peer,
                            max_frame_len: shared.config.max_datagram_size.saturating_sub(25),
                            cleanup: None,
                        },
                    },
                    established: false,
                    auth_decision: Some(decision_sender),
                    security_commands: None,
                    allows_game_controls: false,
                    queued_bytes: ByteBudget::new(shared.config.max_session_queued_bytes),
                },
            )?;
            let mut auth = session_event(EventType::AuthRequest, endpoint, session);
            auth.data.extend_from_slice(&peer_key);
            auth.data.extend_from_slice(&join_payload);
            if shared.events.try_push(auth).is_err() {
                remove_session(shared, endpoint, session);
                return Err(RnetError::new(ErrorCode::WouldBlock, "event queue is full"));
            }
            peers.insert(
                peer,
                SecureUdpPeer::ServerAuth {
                    session,
                    transport,
                    decision,
                    deadline: Instant::now() + HANDSHAKE_TIMEOUT,
                    auth_started: Instant::now(),
                },
            );
        }
        (
            EndpointSecurity::Client { .. },
            UDP_HANDSHAKE_RESPONSE,
            Some(SecureUdpPeer::ClientWaitResponse {
                session,
                mut handshake,
                join_payload,
                handshake_started,
            }),
        ) => {
            if let Err(error) = handshake.read_response(payload) {
                let error = map_security_error(error);
                fail_secure_session(
                    shared,
                    endpoint,
                    session,
                    RnetError::new(error.code(), error.to_string()),
                );
                return Err(error);
            }
            let (finish, transport) = match handshake.finish_datagram(&join_payload) {
                Ok(value) => value,
                Err(error) => {
                    let error = map_security_error(error);
                    fail_secure_session(
                        shared,
                        endpoint,
                        session,
                        RnetError::new(error.code(), error.to_string()),
                    );
                    return Err(error);
                }
            };
            shared
                .latencies
                .record(LatencyKind::CryptoHandshake, handshake_started.elapsed());
            send_control(UDP_HANDSHAKE_FINISH, &finish)?;
            peers.insert(
                peer,
                SecureUdpPeer::ClientWaitDecision {
                    session,
                    transport,
                    auth_started: Instant::now(),
                },
            );
        }
        (
            EndpointSecurity::Client { .. },
            UDP_AUTH_DECISION,
            Some(SecureUdpPeer::ClientWaitDecision {
                session,
                mut transport,
                auth_started,
            }),
        ) => {
            let decision = match transport.decrypt(payload) {
                Ok(decision) => decision,
                Err(error) => {
                    let error = map_security_error(error);
                    fail_secure_session(
                        shared,
                        endpoint,
                        session,
                        RnetError::new(error.code(), error.to_string()),
                    );
                    return Err(error);
                }
            };
            if decision.as_slice() != [1] {
                fail_secure_session(
                    shared,
                    endpoint,
                    session,
                    RnetError::new(ErrorCode::AuthRejected, "server rejected authentication"),
                );
                return Ok(());
            }
            shared
                .latencies
                .record(LatencyKind::AuthWait, auth_started.elapsed());
            mark_session_established(shared, session)?;
            push_tcp_event(
                shared,
                session_event(EventType::SessionOpened, endpoint, session),
            )
            .await;
            peers.insert(peer, SecureUdpPeer::Established { session, transport });
        }
        (
            _,
            UDP_DATA,
            Some(SecureUdpPeer::Established {
                session,
                mut transport,
            }),
        ) => {
            if !session_active(shared, session) {
                return Ok(());
            }
            let plaintext = match transport.decrypt(payload) {
                Ok(plaintext) => plaintext,
                Err(error) => {
                    peers.insert(peer, SecureUdpPeer::Established { session, transport });
                    return Err(map_security_error(error));
                }
            };
            let frame = match decode_datagram(&plaintext, shared.config.max_body_len) {
                Ok(frame) => frame,
                Err(error) => {
                    peers.insert(peer, SecureUdpPeer::Established { session, transport });
                    return Err(error);
                }
            };
            shared
                .metrics
                .frames_received
                .fetch_add(1, Ordering::Relaxed);
            shared
                .metrics
                .bytes_received
                .fetch_add(frame.body.len() as u64, Ordering::Relaxed);
            if shared
                .events
                .try_push(message_event(endpoint, session, frame))
                .is_err()
            {
                shared
                    .metrics
                    .events_dropped
                    .fetch_add(1, Ordering::Relaxed);
            }
            peers.insert(peer, SecureUdpPeer::Established { session, transport });
        }
        (_, _, Some(previous)) => {
            peers.insert(peer, previous);
        }
        _ => {}
    }
    Ok(())
}

type AddressedControlSender<'a> = dyn FnMut(SocketAddr, u8, &[u8]) -> Result<()> + Send + 'a;

pub(crate) async fn process_udp_auth_decisions(
    shared: &Arc<Shared>,
    endpoint: Handle,
    peers: &mut HashMap<SocketAddr, SecureUdpPeer>,
    send_control: &mut AddressedControlSender<'_>,
) {
    let addresses: Vec<_> = peers.keys().copied().collect();
    for peer in addresses {
        let Some(state) = peers.remove(&peer) else {
            continue;
        };
        match state {
            SecureUdpPeer::ServerAuth {
                session,
                transport: _,
                decision: _,
                deadline,
                auth_started: _,
            } if Instant::now() > deadline => {
                fail_secure_session(
                    shared,
                    endpoint,
                    session,
                    RnetError::new(ErrorCode::Timeout, "authentication timed out"),
                );
            }
            SecureUdpPeer::ServerAuth {
                session,
                mut transport,
                mut decision,
                deadline,
                auth_started,
            } => match decision.try_recv() {
                Ok(accepted) => {
                    shared
                        .latencies
                        .record(LatencyKind::AuthWait, auth_started.elapsed());
                    if let Ok(record) = transport.encrypt(&[u8::from(accepted)]) {
                        let _ = send_control(peer, UDP_AUTH_DECISION, &record);
                    }
                    if accepted && mark_session_established(shared, session).is_ok() {
                        push_tcp_event(
                            shared,
                            session_event(EventType::SessionOpened, endpoint, session),
                        )
                        .await;
                        peers.insert(peer, SecureUdpPeer::Established { session, transport });
                    } else {
                        fail_secure_session(
                            shared,
                            endpoint,
                            session,
                            RnetError::new(ErrorCode::AuthRejected, "authentication rejected"),
                        );
                    }
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    peers.insert(
                        peer,
                        SecureUdpPeer::ServerAuth {
                            session,
                            transport,
                            decision,
                            deadline,
                            auth_started,
                        },
                    );
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    remove_session(shared, endpoint, session);
                }
            },
            SecureUdpPeer::ServerWaitFinish { deadline, .. } if Instant::now() > deadline => {}
            other => {
                peers.insert(peer, other);
            }
        }
    }
}

async fn send_udp_control(
    socket: &Arc<UdpSocket>,
    peer: SocketAddr,
    kind: u8,
    payload: &[u8],
) -> Result<()> {
    let mut packet = Vec::with_capacity(1 + payload.len());
    packet.push(kind);
    packet.extend_from_slice(payload);
    socket.send_to(&packet, peer).await?;
    Ok(())
}
