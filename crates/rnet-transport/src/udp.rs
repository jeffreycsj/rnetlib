use crate::metrics::AdmissionRejectReason;
use crate::metrics::LatencyKind;
use crate::state::message_event;
use crate::state::push_endpoint_error;
use crate::state::push_tcp_event;
use crate::state::session_active;
use crate::state::session_event;
use crate::state::ByteBudget;
use crate::state::Outbound;
use crate::state::SessionRoute;
use crate::state::SessionTarget;
use crate::state::Shared;
use rnet_core::ErrorCode;
use rnet_core::EventType;
use rnet_core::Handle;
use rnet_core::Lifecycle;
use rnet_protocol::decode_datagram;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub(crate) async fn run_udp_endpoint(
    shared: Arc<Shared>,
    endpoint: Handle,
    socket: Arc<UdpSocket>,
    sender: mpsc::Sender<(SocketAddr, Outbound)>,
    mut receiver: mpsc::Receiver<(SocketAddr, Outbound)>,
    peers: Arc<Mutex<HashMap<SocketAddr, Handle>>>,
    initial_session: Option<Handle>,
) {
    if let Some(session) = initial_session {
        push_tcp_event(
            &shared,
            session_event(EventType::SessionOpened, endpoint, session),
        )
        .await;
        if shared.state.load() != Lifecycle::Running {
            return;
        }
    }
    let mut buffer = vec![0_u8; shared.config.max_datagram_size + 1];
    loop {
        tokio::select! {
            received = socket.recv_from(&mut buffer) => {
                let (length, peer) = match received {
                    Ok(value) => value,
                    Err(error) => {
                        push_endpoint_error(&shared, endpoint, ErrorCode::IoError, error.to_string());
                        break;
                    }
                };
                if length > shared.config.max_datagram_size {
                    shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let frame = match decode_datagram(&buffer[..length], shared.config.max_body_len) {
                    Ok(frame) => frame,
                    Err(_) => {
                        shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                };
                let existing_session = {
                    let mut peer_map = peers.lock().expect("UDP peer map poisoned");
                    match peer_map.get(&peer).copied() {
                        Some(session) if session_active(&shared, session) => Some(session),
                        Some(_) => {
                            peer_map.remove(&peer);
                            None
                        }
                        None => None,
                    }
                };
                let session = match existing_session {
                    Some(session) => session,
                    None => {
                        let at_capacity = {
                            let mut peer_map = peers.lock().expect("UDP peer map poisoned");
                            peer_map.retain(|_, session| session_active(&shared, *session));
                            peer_map.len() >= shared.config.max_sessions_per_endpoint
                        };
                        if at_capacity {
                            shared.metrics.record_admission_rejected(
                                AdmissionRejectReason::EndpointSessionLimit,
                            );
                            continue;
                        }
                        let session = shared.sessions.lock().expect("session table poisoned").insert(SessionRoute {
                            endpoint,
                            target: SessionTarget::Udp {
                                sender: sender.clone(),
                                peer,
                                max_frame_len: shared.config.max_datagram_size,
                                cleanup: None,
                            },
                            established: true,
                            auth_decision: None,
                            security_commands: None,
                            queued_bytes: ByteBudget::new(shared.config.max_session_queued_bytes),
                        });
                        if shared.events.try_push(session_event(EventType::SessionOpened, endpoint, session)).is_err() {
                            shared.metrics.events_dropped.fetch_add(1, Ordering::Relaxed);
                            let _ = shared
                                .sessions
                                .lock()
                                .expect("session table poisoned")
                                .remove(session);
                            continue;
                        }
                        peers.lock().expect("UDP peer map poisoned").insert(peer, session);
                        session
                    }
                };
                shared.metrics.frames_received.fetch_add(1, Ordering::Relaxed);
                shared.metrics.bytes_received.fetch_add(frame.body.len() as u64, Ordering::Relaxed);
                if shared.events.try_push(message_event(endpoint, session, frame)).is_err() {
                    shared.metrics.events_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            outbound = receiver.recv() => {
                let Some((peer, outbound)) = outbound else { break; };
                shared.latencies.record(
                    LatencyKind::SendQueue,
                    outbound.queued_at.elapsed(),
                );
                match socket.send_to(&outbound.bytes, peer).await {
                    Ok(count) => {
                        shared.metrics.bytes_sent.fetch_add(count as u64, Ordering::Relaxed);
                    }
                    Err(error) => {
                        push_endpoint_error(&shared, endpoint, ErrorCode::IoError, error.to_string());
                    }
                }
            }
        }
    }
}
