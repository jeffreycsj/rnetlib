use crate::admission::AdmissionController;
use crate::metrics::AdmissionRejectReason;
use crate::metrics::LatencyKind;
use crate::record_reader::RecordReader;
use crate::state::fail_secure_session;
use crate::state::insert_session_route;
use crate::state::map_security_error;
use crate::state::mark_session_established;
use crate::state::message_event;
use crate::state::push_endpoint_error;
use crate::state::push_tcp_event;
use crate::state::remove_session;
use crate::state::session_active;
use crate::state::session_event;
use crate::state::ByteBudget;
use crate::state::Outbound;
use crate::state::SessionRoute;
use crate::state::SessionTarget;
use crate::state::Shared;
use crate::tcp_socket::configure_tokio_tcp;
use crate::HANDSHAKE_TIMEOUT;
use crate::MAX_HANDSHAKE_RECORD;
use rnet_core::ErrorCode;
use rnet_core::EventType;
use rnet_core::Handle;
use rnet_core::Lifecycle;
use rnet_core::Result;
use rnet_core::RnetError;
use rnet_protocol::decode_datagram;
use rnet_security::InitiatorHandshake;
use rnet_security::Keypair;
use rnet_security::ResponderHandshake;
use rnet_security::SecureTransport;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::Semaphore;
use tokio::time::timeout;

pub(crate) async fn run_secure_tcp_listener(
    shared: Arc<Shared>,
    endpoint: Handle,
    listener: TcpListener,
    local_key: Keypair,
) {
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
                let (auth_sender, auth_receiver) = oneshot::channel();
                let session = match insert_session_route(
                    &shared,
                    SessionRoute {
                        endpoint,
                        target: SessionTarget::Tcp(sender),
                        established: false,
                        auth_decision: Some(auth_sender),
                        security_commands: None,
                        allows_game_controls: false,
                        queued_bytes: ByteBudget::new(shared.config.max_session_queued_bytes),
                    },
                ) {
                    Ok(session) => session,
                    Err(_) => {
                        drop(stream);
                        continue;
                    }
                };
                let session_shared = Arc::clone(&shared);
                let session_key = local_key.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ip_permit = ip_permit;
                    let result = secure_server_flow(
                        Arc::clone(&session_shared),
                        endpoint,
                        session,
                        stream,
                        receiver,
                        auth_receiver,
                        session_key,
                    )
                    .await;
                    if let Err(error) = result {
                        fail_secure_session(&session_shared, endpoint, session, error);
                    }
                });
            }
            Err(error) => {
                push_endpoint_error(&shared, endpoint, ErrorCode::IoError, error.to_string());
                break;
            }
        }
    }
}

async fn secure_server_flow(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    receiver: mpsc::Receiver<Outbound>,
    auth_receiver: oneshot::Receiver<bool>,
    local_key: Keypair,
) -> Result<()> {
    let handshake_started = Instant::now();
    let mut handshake = ResponderHandshake::new(&local_key.private).map_err(map_security_error)?;
    let first = timeout(
        HANDSHAKE_TIMEOUT,
        read_record(&mut stream, MAX_HANDSHAKE_RECORD),
    )
    .await
    .map_err(|_| RnetError::new(ErrorCode::Timeout, "handshake timed out"))??;
    handshake.read_first(&first).map_err(map_security_error)?;
    let response = handshake.write_response().map_err(map_security_error)?;
    write_record(&mut stream, &response).await?;
    let finish = timeout(
        HANDSHAKE_TIMEOUT,
        read_record(&mut stream, MAX_HANDSHAKE_RECORD),
    )
    .await
    .map_err(|_| RnetError::new(ErrorCode::Timeout, "handshake timed out"))??;
    let (join_payload, peer_key, mut transport) =
        handshake.finish(&finish).map_err(map_security_error)?;
    shared
        .latencies
        .record(LatencyKind::CryptoHandshake, handshake_started.elapsed());

    let mut auth = session_event(EventType::AuthRequest, endpoint, session);
    auth.data.reserve(peer_key.len() + join_payload.len());
    auth.data.extend_from_slice(&peer_key);
    auth.data.extend_from_slice(&join_payload);
    push_tcp_event(&shared, auth).await;

    let auth_started = Instant::now();
    let accepted = timeout(HANDSHAKE_TIMEOUT, auth_receiver)
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "authentication timed out"))?
        .map_err(|_| RnetError::new(ErrorCode::Cancelled, "authentication was cancelled"))?;
    shared
        .latencies
        .record(LatencyKind::AuthWait, auth_started.elapsed());
    let decision = transport
        .encrypt(&[u8::from(accepted)])
        .map_err(map_security_error)?;
    write_record(&mut stream, &decision).await?;
    if !accepted {
        return Err(RnetError::new(
            ErrorCode::AuthRejected,
            "application rejected peer authentication",
        ));
    }
    mark_session_established(&shared, session)?;
    push_tcp_event(
        &shared,
        session_event(EventType::SessionOpened, endpoint, session),
    )
    .await;
    run_secure_tcp_session(shared, endpoint, session, stream, receiver, transport).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_secure_tcp_client(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    receiver: mpsc::Receiver<Outbound>,
    local_key: Keypair,
    expected_server_public: Vec<u8>,
    join_payload: Vec<u8>,
) {
    let result = async {
        let handshake_started = Instant::now();
        let mut handshake = InitiatorHandshake::new(&local_key.private, &expected_server_public)
            .map_err(map_security_error)?;
        let first = handshake.write_first().map_err(map_security_error)?;
        write_record(&mut stream, &first).await?;
        let response = timeout(
            HANDSHAKE_TIMEOUT,
            read_record(&mut stream, MAX_HANDSHAKE_RECORD),
        )
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "handshake timed out"))??;
        handshake
            .read_response(&response)
            .map_err(map_security_error)?;
        let (finish, mut transport) = handshake
            .finish(&join_payload)
            .map_err(map_security_error)?;
        write_record(&mut stream, &finish).await?;
        let decision = timeout(HANDSHAKE_TIMEOUT, read_record(&mut stream, 64))
            .await
            .map_err(|_| RnetError::new(ErrorCode::Timeout, "authentication timed out"))??;
        let plaintext = transport.decrypt(&decision).map_err(map_security_error)?;
        if plaintext.as_slice() != [1] {
            return Err(RnetError::new(
                ErrorCode::AuthRejected,
                "server rejected client authentication",
            ));
        }
        shared
            .latencies
            .record(LatencyKind::CryptoHandshake, handshake_started.elapsed());
        mark_session_established(&shared, session)?;
        push_tcp_event(
            &shared,
            session_event(EventType::SessionOpened, endpoint, session),
        )
        .await;
        run_secure_tcp_session(
            shared.clone(),
            endpoint,
            session,
            stream,
            receiver,
            transport,
        )
        .await;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        fail_secure_session(&shared, endpoint, session, error);
    }
}

async fn run_secure_tcp_session(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    mut receiver: mpsc::Receiver<Outbound>,
    mut transport: SecureTransport,
) {
    let mut records =
        RecordReader::new(shared.config.max_body_len + rnet_protocol::HEADER_LEN + 16);
    loop {
        match records.take() {
            Ok(Some(ciphertext)) => {
                let plaintext = match transport.decrypt(&ciphertext) {
                    Ok(value) => value,
                    Err(_) => {
                        shared
                            .metrics
                            .protocol_errors
                            .fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                };
                let frame = match decode_datagram(&plaintext, shared.config.max_body_len) {
                    Ok(frame) => frame,
                    Err(_) => {
                        shared
                            .metrics
                            .protocol_errors
                            .fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                };
                if !session_active(&shared, session) {
                    break;
                }
                shared
                    .metrics
                    .frames_received
                    .fetch_add(1, Ordering::Relaxed);
                shared
                    .metrics
                    .bytes_received
                    .fetch_add(frame.body.len() as u64, Ordering::Relaxed);
                push_tcp_event(&shared, message_event(endpoint, session, frame)).await;
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                shared
                    .metrics
                    .protocol_errors
                    .fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
        tokio::select! {
            inbound = stream.read_buf(records.buffer_mut()) => {
                match inbound {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            outbound = receiver.recv() => {
                let Some(outbound) = outbound else { break; };
                shared.latencies.record(
                    LatencyKind::SendQueue,
                    outbound.queued_at.elapsed(),
                );
                let frame = outbound.bytes;
                let ciphertext = match transport.encrypt(&frame) {
                    Ok(value) => value,
                    Err(_) => break,
                };
                if write_record(&mut stream, &ciphertext).await.is_err() {
                    break;
                }
                shared.metrics.bytes_sent.fetch_add(frame.len() as u64, Ordering::Relaxed);
                if session_active(&shared, session) {
                    let _ = shared.events.try_push(session_event(EventType::Writable, endpoint, session));
                }
            }
        }
    }
    remove_session(&shared, endpoint, session);
}

async fn read_record(stream: &mut TcpStream, max_len: usize) -> Result<Vec<u8>> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > max_len {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "record length is outside the configured limit",
        ));
    }
    let mut record = vec![0; length];
    stream.read_exact(&mut record).await?;
    Ok(record)
}

async fn write_record(stream: &mut TcpStream, record: &[u8]) -> Result<()> {
    let length = u32::try_from(record.len())
        .map_err(|_| RnetError::new(ErrorCode::MessageTooLarge, "record is too large"))?;
    stream.write_u32(length).await?;
    stream.write_all(record).await?;
    Ok(())
}
