//! One fair UDP/KCP endpoint loop owns peer state, cookie preflight, and deferred sends.
//!
//! Source addresses are attacker-controlled until the stateless cookie and Noise handshake
//! complete. Durable peer/KCP state and application sessions are therefore allocated only after
//! preflight, and a slow peer's deferred queue is bounded by the endpoint write capacity.

use crate::adaptive_codec::data_record;
use crate::adaptive_datagram_maintenance::{cleanup_peer, drive_server};
use crate::adaptive_datagram_process::process;
use crate::adaptive_datagram_send::flush_deferred;
use crate::adaptive_kcp_preflight::handle_preflight;
use crate::adaptive_peer::Peer;
use crate::adaptive_wire::DatagramWire;
use crate::admission::{AdmissionController, AdmissionPermit};
use crate::auto_rekey::AutoRekey;
use crate::config::{ClientSecurity, EndpointSecurity};
use crate::cookie::{CookieGuard, COOKIE_LEN};
use crate::kcp_preflight::encode_hello;
use crate::state::{
    fail_secure_session, push_endpoint_error, remove_session_with_reason,
    retarget_datagram_endpoint, retarget_datagram_session, session_active, DatagramCleanup,
    Outbound, Shared,
};
use rnet_core::{ErrorCode, Handle, Result, RnetError, Transport};
use rnet_protocol::control::{decode_record, Record, RecordKind, SecurityMode};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_adaptive_datagram(
    shared: Arc<Shared>,
    endpoint: Handle,
    socket: Arc<UdpSocket>,
    sender: mpsc::Sender<(SocketAddr, Outbound)>,
    mut receiver: mpsc::Receiver<(SocketAddr, Outbound)>,
    cleanup_sender: mpsc::UnboundedSender<DatagramCleanup>,
    mut cleanup_receiver: mpsc::UnboundedReceiver<DatagramCleanup>,
    remote: Option<SocketAddr>,
    initial_session: Option<Handle>,
    security: EndpointSecurity,
    client_security: Option<ClientSecurity>,
    transport_kind: Transport,
    remote_candidates: Vec<SocketAddr>,
    connect_deadline: Option<Instant>,
) {
    let mut socket = socket;
    let mut remote = remote;
    let mut remote_candidates = VecDeque::from(remote_candidates);
    let cookie = match CookieGuard::new() {
        Ok(value) => value,
        Err(_) => return,
    };
    let mut peers = HashMap::new();
    let mut deferred = HashMap::<SocketAddr, VecDeque<Outbound>>::new();
    let mut deferred_count = 0usize;
    let mut deferred_peers = Vec::new();
    let mut wire = DatagramWire::new_with_limits(
        transport_kind,
        shared.config.max_datagram_size,
        shared.config.max_sessions_per_endpoint,
        shared.config.max_session_queued_bytes,
        shared.config.max_runtime_queued_bytes,
    );
    let mut preflight_authorized = HashMap::<SocketAddr, Instant>::new();
    let per_ip = AdmissionController::new(
        shared.config.max_sessions_per_ip,
        shared.config.handshake_rate_per_ip,
        shared.config.handshake_burst_per_ip,
        shared.config.max_sessions_per_endpoint,
        shared.config.ipv6_admission_prefix_bits,
    );
    let mut admission_permits = HashMap::<SocketAddr, AdmissionPermit>::new();
    let mut automatic_rekeys = HashMap::<SocketAddr, AutoRekey>::new();
    if let (Some(peer), Some(session)) = (remote, initial_session) {
        let started = send_client_hello(&socket, &mut wire, transport_kind, peer).await;
        if let Err(error) = started {
            fail_secure_session(&shared, endpoint, session, error);
            return;
        }
        peers.insert(
            peer,
            Peer::ClientHello {
                session,
                started: Instant::now(),
            },
        );
    }
    let mut buffer = vec![0; shared.config.max_datagram_size + 1];
    let mut tick = tokio::time::interval(Duration::from_millis(5));
    loop {
        tokio::select! {
            inbound = socket.recv_from(&mut buffer) => {
                let (length, peer) = match inbound { Ok(v) => v, Err(_) => break };
                // Cheap cookie/admission checks precede KCP allocation and authenticated record
                // parsing. This is the DoS boundary for unknown datagram sources.
                match handle_preflight(
                    &shared, &socket, &mut wire, transport_kind, &security, remote, &cookie,
                    &per_ip, &mut admission_permits, &mut preflight_authorized, peer,
                    &buffer[..length],
                ).await {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => {
                        if let Some(session) = initial_session {
                            fail_secure_session(&shared, endpoint, session, error);
                        }
                        return;
                    }
                }
                let delivered = match wire.receive(peer, &buffer[..length]) {
                    Ok(records) => records,
                    Err(_) => { shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed); continue; }
                };
                if wire.has_peer(peer) {
                    preflight_authorized.remove(&peer);
                }
                for bytes in delivered {
                    let record_limit = if transport_kind == Transport::Kcp {
                        shared
                            .config
                            .max_body_len
                            .saturating_add(rnet_protocol::HEADER_LEN + 64)
                    } else {
                        shared.config.max_datagram_size
                    };
                    let record = match decode_record(&bytes, record_limit) {
                        Ok(v) => v, Err(_) => { shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed); continue; }
                    };
                    if matches!(security, EndpointSecurity::AdaptiveServer { .. })
                        && record.kind == RecordKind::ClientHello
                        && !peers.contains_key(&peer)
                        && !admission_permits.contains_key(&peer)
                        && record.payload.len() == COOKIE_LEN
                        && cookie.validate(peer, &record.payload)
                    {
                        let permit = match per_ip.try_acquire(peer.ip()) {
                            Ok(permit) => permit,
                            Err(reason) => {
                                shared
                                    .metrics
                                    .record_admission_rejected(reason.metric_reason());
                                continue;
                            }
                        };
                        admission_permits.insert(peer, permit);
                    }
                    if record.kind != RecordKind::PlainData {
                        let (duplicate, response) = wire.begin_input(peer, &bytes);
                        if duplicate {
                            if let Some(response) = response {
                                let _ = socket.send_to(&response, peer).await;
                            }
                            continue;
                        }
                    }
                    let prior_session = peers.get(&peer).and_then(Peer::session);
                    let processed = process(&shared, endpoint, &socket, &mut wire, &sender, &security, client_security.as_ref(),
                        &cleanup_sender, transport_kind, &cookie, &mut peers, peer, record).await;
                    let success = processed.is_ok();
                    if success
                        && matches!(
                            peers.get(&peer),
                            Some(Peer::Established {
                                commands: Some(_),
                                ..
                            })
                        )
                    {
                        automatic_rekeys.entry(peer).or_insert_with(|| {
                            AutoRekey::new(
                                shared.config.security_policy.rekey_after,
                                shared.config.security_policy.rekey_after_bytes,
                                Instant::now(),
                            )
                        });
                    }
                    if let Err(error) = processed {
                        shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                        if !peers.contains_key(&peer) {
                            let session = prior_session.or_else(|| {
                                (remote == Some(peer)).then_some(initial_session).flatten()
                            });
                            if let Some(session) = session.filter(|session| session_active(&shared, *session)) {
                                if remote == Some(peer) {
                                    fail_secure_session(&shared, endpoint, session, error);
                                } else {
                                    remove_session_with_reason(
                                        &shared,
                                        endpoint,
                                        session,
                                        error.code(),
                                    );
                                }
                            }
                            if let Some(queued) = deferred.remove(&peer) {
                                deferred_count = deferred_count.saturating_sub(queued.len());
                            }
                            wire.remove_peer(peer);
                            admission_permits.remove(&peer);
                            automatic_rekeys.remove(&peer);
                        }
                    }
                    wire.end_input(success);
                }
            }
            outbound = receiver.recv(), if deferred_count < shared.config.write_queue_capacity => {
                let Some((peer, outbound)) = outbound else { break };
                let Some(mut state) = peers.remove(&peer) else { continue };
                if let Peer::Established {
                    session,
                    transport,
                    controller,
                    commands: _,
                    last_activity,
                    ..
                } = &mut state {
                    if session_active(&shared, *session) && !controller.is_transitioning() {
                        let record = match data_record(
                            transport,
                            controller,
                            &outbound.bytes,
                            shared.config.max_body_len,
                        ) {
                            Ok(record) => record,
                            Err(error) => {
                                shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                                remove_session_with_reason(
                                    &shared,
                                    endpoint,
                                    *session,
                                    error.code(),
                                );
                                continue;
                            }
                        };
                        match wire
                            .send(&socket, peer, &record, shared.config.max_datagram_size)
                            .await
                        {
                            Ok(()) => {
                                shared.latencies.record(
                                    crate::metrics::LatencyKind::SendQueue,
                                    outbound.queued_at.elapsed(),
                                );
                            }
                            Err(error) if error.code() == ErrorCode::WouldBlock => {
                                deferred.entry(peer).or_default().push_back(outbound);
                                deferred_count += 1;
                                peers.insert(peer, state);
                                continue;
                            }
                            Err(error) => {
                                remove_session_with_reason(
                                    &shared,
                                    endpoint,
                                    *session,
                                    error.code(),
                                );
                                continue;
                            }
                        }
                        shared.metrics.bytes_sent.fetch_add(
                            outbound.bytes.len() as u64,
                            Ordering::Relaxed,
                        );
                        if controller.mode() == SecurityMode::Encrypted {
                            if let Some(rekey) = automatic_rekeys.get_mut(&peer) {
                                rekey.record_encrypted_bytes(outbound.bytes.len());
                            }
                        }
                        *last_activity = Instant::now();
                    } else if session_active(&shared, *session) {
                        deferred.entry(peer).or_default().push_back(outbound);
                        deferred_count += 1;
                    }
                }
                peers.insert(peer, state);
            }
            cleanup = cleanup_receiver.recv() => {
                let Some(cleanup) = cleanup else { break };
                cleanup_peer(
                    &mut peers,
                    &mut deferred,
                    &mut deferred_count,
                    &mut wire,
                    cleanup.peer,
                    cleanup.session,
                );
                admission_permits.remove(&cleanup.peer);
                automatic_rekeys.remove(&cleanup.peer);
            }
            scheduled = tick.tick() => {
                let now = Instant::now();
                if let Some(current) = remote {
                    if connect_deadline.is_some_and(|deadline| now >= deadline)
                        && peers
                            .get(&current)
                            .is_some_and(|state| !state.is_established())
                    {
                        if let Some(session) = peers.get(&current).and_then(Peer::session) {
                            fail_secure_session(
                                &shared,
                                endpoint,
                                session,
                                RnetError::new(
                                    ErrorCode::Timeout,
                                    "datagram multi-address connection deadline expired",
                                ),
                            );
                        }
                        return;
                    }
                    if let Some((session, next)) = take_retry_candidate(
                        peers.get(&current),
                        shared.config.handshake_timeout,
                        &mut remote_candidates,
                    ) {
                        push_endpoint_error(
                            &shared,
                            endpoint,
                            ErrorCode::Timeout,
                            format!(
                                "datagram candidate {current} timed out; retrying {next}"
                            ),
                        );
                        peers.remove(&current);
                        wire.remove_peer(current);
                        preflight_authorized.remove(&current);
                        admission_permits.remove(&current);
                        automatic_rekeys.remove(&current);
                        if socket.local_addr().is_ok_and(|local| {
                            local.is_ipv4() != next.is_ipv4()
                        }) {
                            let bind_addr = wildcard_for(next);
                            match UdpSocket::bind(bind_addr).await {
                                Ok(rebound) => {
                                    let local_addr = match rebound.local_addr() {
                                        Ok(local_addr) => local_addr,
                                        Err(error) => {
                                            fail_secure_session(
                                                &shared,
                                                endpoint,
                                                session,
                                                error.into(),
                                            );
                                            return;
                                        }
                                    };
                                    if let Err(error) = retarget_datagram_endpoint(
                                        &shared,
                                        endpoint,
                                        local_addr,
                                    ) {
                                        fail_secure_session(&shared, endpoint, session, error);
                                        return;
                                    }
                                    socket = Arc::new(rebound);
                                }
                                Err(error) => {
                                    fail_secure_session(
                                        &shared,
                                        endpoint,
                                        session,
                                        error.into(),
                                    );
                                    return;
                                }
                            }
                        }
                        if let Err(error) = retarget_datagram_session(&shared, session, next) {
                            fail_secure_session(&shared, endpoint, session, error);
                            return;
                        }
                        if let Err(error) =
                            send_client_hello(&socket, &mut wire, transport_kind, next).await
                        {
                            fail_secure_session(&shared, endpoint, session, error);
                            return;
                        }
                        remote = Some(next);
                        peers.insert(
                            next,
                            Peer::ClientHello {
                                session,
                                started: Instant::now(),
                            },
                        );
                    }
                }
                preflight_authorized.retain(|peer, deadline| {
                    if now < *deadline {
                        true
                    } else {
                        wire.revoke_authorization(*peer);
                        admission_permits.remove(peer);
                        false
                    }
                });
                if transport_kind == Transport::Kcp {
                    shared.latencies.record(
                        crate::metrics::LatencyKind::KcpUpdateDelay,
                        tokio::time::Instant::now().saturating_duration_since(scheduled),
                    );
                }
                drive_server(
                    &shared,
                    endpoint,
                    &socket,
                    &mut wire,
                    &mut peers,
                    &mut deferred_peers,
                    &mut automatic_rekeys,
                    remote_candidates.is_empty(),
                )
                .await;
                admission_permits.retain(|peer, _| {
                    peers.contains_key(peer) || preflight_authorized.contains_key(peer)
                });
                automatic_rekeys.retain(|peer, _| peers.contains_key(peer));
                flush_deferred(
                    &shared, &socket, &mut wire, &mut peers, &mut deferred,
                    &mut deferred_count, &mut deferred_peers, &mut automatic_rekeys,
                ).await;
                wire.flush(&socket, &shared).await;
                for peer in wire.retry(&socket).await {
                    let Some(state) = peers.remove(&peer) else { continue };
                    if let Some(queued) = deferred.remove(&peer) {
                        deferred_count = deferred_count.saturating_sub(queued.len());
                    }
                    if let Some(session) = state.session() {
                        if remote == Some(peer) && !state.is_established() {
                            fail_secure_session(
                                &shared,
                                endpoint,
                                session,
                                RnetError::new(ErrorCode::Timeout, "datagram handshake timed out"),
                            );
                        } else {
                            remove_session_with_reason(
                                &shared,
                                endpoint,
                                session,
                                ErrorCode::Timeout,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn wildcard_for(peer: SocketAddr) -> SocketAddr {
    SocketAddr::new(
        if peer.is_ipv4() {
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        } else {
            std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
        },
        0,
    )
}

async fn send_client_hello(
    socket: &UdpSocket,
    wire: &mut DatagramWire,
    transport: Transport,
    peer: SocketAddr,
) -> Result<()> {
    if transport == Transport::Kcp {
        socket.send_to(&encode_hello(&[]), peer).await?;
        Ok(())
    } else {
        wire.send_reliable(
            socket,
            peer,
            &Record::new(RecordKind::ClientHello, 0, &[]),
            128,
        )
        .await
    }
}

fn pending_client_timeout(state: &Peer, timeout: Duration) -> Option<Handle> {
    match state {
        Peer::ClientHello { session, started }
        | Peer::ClientResponse {
            session, started, ..
        } if started.elapsed() >= timeout => Some(*session),
        Peer::ClientAuth {
            session,
            connect_started,
            ..
        } if connect_started.elapsed() >= timeout => Some(*session),
        _ => None,
    }
}

fn take_retry_candidate(
    state: Option<&Peer>,
    timeout: Duration,
    candidates: &mut VecDeque<SocketAddr>,
) -> Option<(Handle, SocketAddr)> {
    let session = pending_client_timeout(state?, timeout)?;
    Some((session, candidates.pop_front()?))
}

#[cfg(test)]
mod tests {
    use super::take_retry_candidate;
    use crate::adaptive_peer::Peer;
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    #[test]
    fn retry_candidate_is_not_consumed_before_the_handshake_timeout() {
        let state = Peer::ClientHello {
            session: 7,
            started: Instant::now(),
        };
        let next = "127.0.0.1:9001".parse().unwrap();
        let mut candidates = VecDeque::from([next]);

        assert_eq!(
            take_retry_candidate(Some(&state), Duration::from_secs(1), &mut candidates),
            None
        );
        assert_eq!(candidates, VecDeque::from([next]));
        assert_eq!(
            take_retry_candidate(Some(&state), Duration::ZERO, &mut candidates),
            Some((7, next))
        );
        assert!(candidates.is_empty());
    }
}
