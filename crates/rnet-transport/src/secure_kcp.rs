use crate::config::EndpointSecurity;
use crate::cookie::CookieGuard;
use crate::kcp::KcpEngine;
use crate::kcp::RustKcpEngine;
use crate::metrics::LatencyKind;
use crate::secure_datagram::handle_secure_udp_packet;
use crate::secure_datagram::process_udp_auth_decisions;
use crate::secure_datagram::SecureUdpPeer;
use crate::secure_datagram::MAX_SECURE_DATAGRAM_PEERS;
use crate::secure_datagram::UDP_DATA;
use crate::secure_datagram::UDP_HANDSHAKE_FIRST;
use crate::state::session_active;
use crate::state::Outbound;
use crate::state::Shared;
use bytes::BytesMut;
use rnet_core::Handle;
use rnet_core::Transport;
use rnet_security::InitiatorHandshake;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

const KCP_CONV: u32 = 0x524e_4554;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_secure_kcp_endpoint(
    shared: Arc<Shared>,
    endpoint: Handle,
    socket: Arc<UdpSocket>,
    sender: mpsc::Sender<(SocketAddr, Outbound)>,
    mut receiver: mpsc::Receiver<(SocketAddr, Outbound)>,
    remote_addr: Option<SocketAddr>,
    initial_session: Option<Handle>,
    security: EndpointSecurity,
) {
    let mut engines = HashMap::<SocketAddr, RustKcpEngine>::new();
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
        if let Ok(mut handshake) =
            InitiatorHandshake::new(&local_key.private, expected_server_public)
        {
            if let Ok(first) = handshake.write_first() {
                if let Ok(mut engine) = RustKcpEngine::new(KCP_CONV) {
                    let control = encode_control(UDP_HANDSHAKE_FIRST, &first);
                    if engine.send(&control).is_ok() {
                        engines.insert(peer, engine);
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
            }
        }
    }

    let started = Instant::now();
    let mut buffer = vec![0_u8; shared.config.max_datagram_size + 1];
    let kcp_interval = Duration::from_millis(10);
    let mut tick =
        tokio::time::interval_at(tokio::time::Instant::now() + kcp_interval, kcp_interval);
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
                let at_capacity = engines.len() >= MAX_SECURE_DATAGRAM_PEERS;
                if let std::collections::hash_map::Entry::Vacant(entry) = engines.entry(peer) {
                    if at_capacity {
                        continue;
                    }
                    match RustKcpEngine::new(KCP_CONV) {
                        Ok(engine) => {
                            entry.insert(engine);
                        }
                        Err(_) => continue,
                    }
                }
                let now = started.elapsed().as_millis() as u64;
                let mut delivered = Vec::<Vec<u8>>::new();
                let input_ok = {
                    let engine = engines.get_mut(&peer).expect("KCP peer exists");
                    if engine.input(&buffer[..length], now).is_err() {
                        false
                    } else {
                        if let Some(rtt) = engine.observed_rtt() {
                            shared.latencies.record(LatencyKind::KcpRtt, rtt);
                        }
                        loop {
                            let mut message = BytesMut::new();
                            match engine.recv(&mut message) {
                                Ok(Some(_)) => delivered.push(message.to_vec()),
                                Ok(None) => break,
                                Err(_) => break,
                            }
                        }
                        true
                    }
                };
                if !input_ok {
                    shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                for control in delivered {
                    if control.is_empty() {
                        continue;
                    }
                    let mut outgoing = Vec::<(u8, Vec<u8>)>::new();
                    let handled = handle_secure_udp_packet(
                        &shared,
                        endpoint,
                        &sender,
                        &security,
                        &cookie_guard,
                        Transport::Kcp,
                        &mut peers,
                        peer,
                        control[0],
                        &control[1..],
                        &mut |kind, payload| {
                            outgoing.push((kind, payload.to_vec()));
                            Ok(())
                        },
                    ).await;
                    if handled.is_err() {
                        shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(engine) = engines.get_mut(&peer) {
                        for (kind, payload) in outgoing {
                            let _ = engine.send(&encode_control(kind, &payload));
                        }
                    }
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
                        if let (Ok(record), Some(engine)) =
                            (transport.encrypt(&frame), engines.get_mut(&peer))
                        {
                            let _ = engine.send(&encode_control(UDP_DATA, &record));
                            shared.metrics.bytes_sent.fetch_add(frame.len() as u64, Ordering::Relaxed);
                        }
                        peers.insert(peer, SecureUdpPeer::Established { session, transport });
                    }
                    other => { peers.insert(peer, other); }
                }
            }
            scheduled = tick.tick() => {
                shared.latencies.record(
                    LatencyKind::KcpUpdateDelay,
                    tokio::time::Instant::now().saturating_duration_since(scheduled),
                );
                let mut controls = Vec::<(SocketAddr, u8, Vec<u8>)>::new();
                process_udp_auth_decisions(
                    &shared,
                    endpoint,
                    &mut peers,
                    &mut |peer, kind, payload| {
                        controls.push((peer, kind, payload.to_vec()));
                        Ok(())
                    },
                ).await;
                for (peer, kind, payload) in controls {
                    if let Some(engine) = engines.get_mut(&peer) {
                        let _ = engine.send(&encode_control(kind, &payload));
                    }
                }
                let now = started.elapsed().as_millis() as u64;
                let addresses: Vec<_> = engines.keys().copied().collect();
                let mut packets = Vec::<(SocketAddr, Vec<u8>)>::new();
                for peer in addresses {
                    if let Some(engine) = engines.get_mut(&peer) {
                        engine.update(now, &mut |packet| packets.push((peer, packet.to_vec())));
                    }
                }
                for (peer, packet) in packets {
                    let _ = socket.send_to(&packet, peer).await;
                }
            }
        }
    }
}

fn encode_control(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut control = Vec::with_capacity(1 + payload.len());
    control.push(kind);
    control.extend_from_slice(payload);
    control
}
