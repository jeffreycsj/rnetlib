use crate::admission::AdmissionController;
use crate::metrics::AdmissionRejectReason;
use crate::metrics::LatencyKind;
use crate::state::message_event;
use crate::state::push_endpoint_error;
use crate::state::push_tcp_event;
use crate::state::remove_session_with_reason;
use crate::state::session_active;
use crate::state::session_event;
use crate::state::ByteBudget;
use crate::state::Outbound;
use crate::state::SessionRoute;
use crate::state::SessionTarget;
use crate::state::Shared;
use crate::tcp_socket::configure_tokio_tcp;
use bytes::BytesMut;
use rnet_core::ErrorCode;
use rnet_core::EventType;
use rnet_core::Handle;
use rnet_core::Lifecycle;
use rnet_protocol::FrameCodec;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Semaphore};

pub(crate) async fn run_tcp_listener(shared: Arc<Shared>, endpoint: Handle, listener: TcpListener) {
    let admission = Arc::new(Semaphore::new(shared.config.max_sessions_per_endpoint));
    let per_ip = AdmissionController::new(
        shared.config.max_sessions_per_ip,
        shared.config.handshake_rate_per_ip,
        shared.config.handshake_burst_per_ip,
        shared.config.max_sessions_per_endpoint.saturating_mul(2),
        shared.config.ipv6_admission_prefix_bits,
    );
    while shared.state.load() == Lifecycle::Running {
        match listener.accept().await {
            Ok((stream, remote)) => {
                if let Err(error) = configure_tokio_tcp(&stream, &shared.config) {
                    push_endpoint_error(&shared, endpoint, error.code(), error.to_string());
                    continue;
                }
                let Ok(permit) = Arc::clone(&admission).try_acquire_owned() else {
                    shared
                        .metrics
                        .record_admission_rejected(AdmissionRejectReason::EndpointSessionLimit);
                    drop(stream);
                    continue;
                };
                let ip_permit = match per_ip.try_acquire(remote.ip()) {
                    Ok(permit) => permit,
                    Err(reason) => {
                        shared
                            .metrics
                            .record_admission_rejected(reason.metric_reason());
                        drop(stream);
                        continue;
                    }
                };
                let (sender, receiver) = mpsc::channel(shared.config.write_queue_capacity);
                let session = shared
                    .sessions
                    .lock()
                    .expect("session table poisoned")
                    .insert(SessionRoute {
                        endpoint,
                        target: SessionTarget::Tcp(sender),
                        established: true,
                        auth_decision: None,
                        security_commands: None,
                        queued_bytes: ByteBudget::new(shared.config.max_session_queued_bytes),
                    });
                push_tcp_event(
                    &shared,
                    session_event(EventType::SessionOpened, endpoint, session),
                )
                .await;
                let session_shared = Arc::clone(&shared);
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ip_permit = ip_permit;
                    run_tcp_session(session_shared, endpoint, session, stream, receiver).await;
                });
            }
            Err(error) => {
                push_endpoint_error(&shared, endpoint, ErrorCode::IoError, error.to_string());
                break;
            }
        }
    }
}

pub(crate) async fn run_tcp_session(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    mut receiver: mpsc::Receiver<Outbound>,
) {
    let mut codec = FrameCodec::new(shared.config.max_body_len).expect("validated max body");
    let mut read_buffer = BytesMut::with_capacity(8192);
    let reason = loop {
        tokio::select! {
            read = stream.read_buf(&mut read_buffer) => {
                match read {
                    Ok(0) => break ErrorCode::IoError,
                    Ok(count) => {
                        let input = read_buffer.split_to(count);
                        match codec.push(&input) {
                            Ok(frames) => {
                                for frame in frames {
                                    if !session_active(&shared, session) {
                                        break;
                                    }
                                    shared.metrics.frames_received.fetch_add(1, Ordering::Relaxed);
                                    shared.metrics.bytes_received.fetch_add(frame.body.len() as u64, Ordering::Relaxed);
                                    push_tcp_event(&shared, message_event(endpoint, session, frame)).await;
                                }
                            }
                            Err(error) => {
                                shared.metrics.protocol_errors.fetch_add(1, Ordering::Relaxed);
                                push_endpoint_error(&shared, endpoint, error.code(), error.to_string());
                                break error.code();
                            }
                        }
                    }
                    Err(error) => {
                        push_endpoint_error(&shared, endpoint, ErrorCode::IoError, error.to_string());
                        break ErrorCode::IoError;
                    }
                }
            }
            outbound = receiver.recv() => {
                let Some(outbound) = outbound else { break ErrorCode::Cancelled; };
                shared.latencies.record(
                    LatencyKind::SendQueue,
                    outbound.queued_at.elapsed(),
                );
                if let Err(error) = stream.write_all(&outbound.bytes).await {
                    push_endpoint_error(&shared, endpoint, ErrorCode::IoError, error.to_string());
                    break ErrorCode::IoError;
                }
                shared.metrics.bytes_sent.fetch_add(outbound.bytes.len() as u64, Ordering::Relaxed);
                if session_active(&shared, session) {
                    let _ = shared.events.try_push(session_event(EventType::Writable, endpoint, session));
                }
            }
        }
    };
    remove_session_with_reason(&shared, endpoint, session, reason);
}
