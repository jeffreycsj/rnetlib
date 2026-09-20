//! TCP listener and Noise/application-authentication handshake for unified sessions.
//!
//! A TCP accept is deliberately only a pending route. The application sees `SessionOpened` after
//! cryptographic peer verification and its own authorization decision; rejected peers never gain
//! access to the business send path.

use crate::adaptive_tcp_session::run_adaptive_session;
use crate::admission::AdmissionController;
use crate::config::ClientSecurity;
use crate::metrics::AdmissionRejectReason;
use crate::metrics::LatencyKind;
use crate::state::{
    fail_secure_session, insert_session_route, map_security_error, mark_session_established,
    push_endpoint_error, push_tcp_event, session_event, ByteBudget, Outbound, SecurityCommand,
    SessionRoute, SessionTarget, Shared,
};
use crate::tcp_socket::configure_tokio_tcp;
use crate::MAX_HANDSHAKE_RECORD;
use rnet_core::{ErrorCode, EventType, Handle, Lifecycle, Result, RnetError};
use rnet_protocol::control::{
    decode_protected, decode_record, encode_protected, encode_record, ProtectedKind,
    ProtectedMessage, Record, RecordKind, SecurityMode,
};
use rnet_security::transition::{Role, SecurityController};
use rnet_security::{InitiatorHandshake, Keypair, ResponderHandshake, SecureTransport};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::time::timeout;

const INITIAL_EPOCH: u64 = 1;
const SERVER_HELLO_LEN: usize = 33;
pub(crate) async fn run_adaptive_tcp_listener(
    shared: Arc<Shared>,
    endpoint: Handle,
    listener: TcpListener,
    local_key: Keypair,
    initial_mode: SecurityMode,
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
                // Permits live for the spawned session task, not just the socket accept. This
                // bounds both expensive handshakes and long-lived peers per listener/IP.
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
                let (security_sender, security_receiver) = mpsc::channel(1);
                let session = match insert_session_route(
                    &shared,
                    SessionRoute {
                        endpoint,
                        target: SessionTarget::Tcp(sender),
                        established: false,
                        auth_decision: Some(auth_sender),
                        security_commands: Some(security_sender),
                        allows_game_controls: true,
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
                    let result = adaptive_server_flow(
                        Arc::clone(&session_shared),
                        endpoint,
                        session,
                        stream,
                        receiver,
                        security_receiver,
                        auth_receiver,
                        session_key,
                        initial_mode,
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

#[allow(clippy::too_many_arguments)]
async fn adaptive_server_flow(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    receiver: mpsc::Receiver<Outbound>,
    security_receiver: mpsc::Receiver<SecurityCommand>,
    auth_receiver: oneshot::Receiver<bool>,
    local_key: Keypair,
    initial_mode: SecurityMode,
) -> Result<()> {
    let started = Instant::now();
    expect_kind(
        timeout(
            shared.config.handshake_timeout,
            read_wire(&mut stream, MAX_HANDSHAKE_RECORD),
        )
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "client hello timed out"))??,
        RecordKind::ClientHello,
    )?;
    let mut hello = Vec::with_capacity(SERVER_HELLO_LEN);
    hello.push(initial_mode as u8);
    hello.extend_from_slice(&local_key.public);
    write_wire(
        &mut stream,
        &Record::new(RecordKind::ServerHello, 0, &hello),
        MAX_HANDSHAKE_RECORD,
    )
    .await?;

    let mut handshake = ResponderHandshake::new(&local_key.private).map_err(map_security_error)?;
    let first = expect_kind(
        timeout(
            shared.config.handshake_timeout,
            read_wire(&mut stream, MAX_HANDSHAKE_RECORD),
        )
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "handshake timed out"))??,
        RecordKind::Handshake,
    )?;
    handshake
        .read_first(&first.payload)
        .map_err(map_security_error)?;
    let response = handshake.write_response().map_err(map_security_error)?;
    write_wire(
        &mut stream,
        &Record::new(RecordKind::Handshake, 0, &response),
        MAX_HANDSHAKE_RECORD,
    )
    .await?;
    let finish = expect_kind(
        timeout(
            shared.config.handshake_timeout,
            read_wire(&mut stream, MAX_HANDSHAKE_RECORD),
        )
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "handshake timed out"))??,
        RecordKind::Handshake,
    )?;
    let (join_payload, peer_key, mut transport) = handshake
        .finish(&finish.payload)
        .map_err(map_security_error)?;
    shared
        .latencies
        .record(LatencyKind::CryptoHandshake, started.elapsed());

    // The join payload is authenticated by Noise, but only game/application policy can decide
    // whether that identity and ticket may become an established business session.
    let mut auth = session_event(EventType::AuthRequest, endpoint, session);
    auth.data.extend_from_slice(&peer_key);
    auth.data.extend_from_slice(&join_payload);
    push_tcp_event(&shared, auth).await;
    let auth_started = Instant::now();
    let accepted = timeout(shared.config.handshake_timeout, auth_receiver)
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "authentication timed out"))?
        .map_err(|_| RnetError::new(ErrorCode::Cancelled, "authentication was cancelled"))?;
    shared
        .latencies
        .record(LatencyKind::AuthWait, auth_started.elapsed());
    write_protected(
        &mut stream,
        &mut transport,
        INITIAL_EPOCH,
        ProtectedKind::AuthDecision,
        &[u8::from(accepted), initial_mode as u8],
        64,
    )
    .await?;
    if !accepted {
        return Err(RnetError::new(
            ErrorCode::AuthRejected,
            "authentication rejected",
        ));
    }
    // Publish readiness only after the encrypted authorization decision has been sent. In
    // particular, a connected TCP socket must never appear as an authorized game session.
    mark_session_established(&shared, session)?;
    push_tcp_event(
        &shared,
        session_event(EventType::SessionOpened, endpoint, session),
    )
    .await;
    run_adaptive_session(
        shared,
        endpoint,
        session,
        stream,
        receiver,
        Some(security_receiver),
        transport,
        SecurityController::new(Role::Server, initial_mode, INITIAL_EPOCH),
    )
    .await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_adaptive_tcp_client(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    receiver: mpsc::Receiver<Outbound>,
    client_security: ClientSecurity,
    join_payload: Vec<u8>,
) {
    let result = async {
        let started = Instant::now();
        write_wire(
            &mut stream,
            &Record::new(RecordKind::ClientHello, 0, &[]),
            MAX_HANDSHAKE_RECORD,
        )
        .await?;
        let hello = expect_kind(
            timeout(
                shared.config.handshake_timeout,
                read_wire(&mut stream, MAX_HANDSHAKE_RECORD),
            )
            .await
            .map_err(|_| RnetError::new(ErrorCode::Timeout, "server hello timed out"))??,
            RecordKind::ServerHello,
        )?;
        if hello.payload.len() != SERVER_HELLO_LEN {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "invalid server hello",
            ));
        }
        let initial_mode = SecurityMode::try_from(hello.payload[0])?;
        let server_key: [u8; 32] = hello.payload[1..]
            .try_into()
            .expect("validated server key length");
        if !(client_security.peer_verifier)(&server_key) {
            return Err(RnetError::new(
                ErrorCode::PeerKeyMismatch,
                "server identity verifier rejected the key",
            ));
        }
        let mut handshake =
            InitiatorHandshake::new(&client_security.local_key.private, &server_key)
                .map_err(map_security_error)?;
        let first = handshake.write_first().map_err(map_security_error)?;
        write_wire(
            &mut stream,
            &Record::new(RecordKind::Handshake, 0, &first),
            MAX_HANDSHAKE_RECORD,
        )
        .await?;
        let response = expect_kind(
            timeout(
                shared.config.handshake_timeout,
                read_wire(&mut stream, MAX_HANDSHAKE_RECORD),
            )
            .await
            .map_err(|_| RnetError::new(ErrorCode::Timeout, "handshake timed out"))??,
            RecordKind::Handshake,
        )?;
        handshake
            .read_response(&response.payload)
            .map_err(map_security_error)?;
        let (finish, mut transport) = handshake
            .finish(&join_payload)
            .map_err(map_security_error)?;
        write_wire(
            &mut stream,
            &Record::new(RecordKind::Handshake, 0, &finish),
            MAX_HANDSHAKE_RECORD,
        )
        .await?;
        let auth_started = Instant::now();
        let decision = timeout(
            shared.config.handshake_timeout,
            read_protected(&mut stream, &mut transport, 64),
        )
        .await
        .map_err(|_| RnetError::new(ErrorCode::Timeout, "authentication timed out"))??;
        shared
            .latencies
            .record(LatencyKind::AuthWait, auth_started.elapsed());
        if decision.0 != INITIAL_EPOCH || decision.1.kind != ProtectedKind::AuthDecision {
            return Err(RnetError::new(
                ErrorCode::AuthRejected,
                "authentication rejected",
            ));
        }
        match decision.1.payload.as_slice() {
            [1, authenticated_mode] if *authenticated_mode == initial_mode as u8 => {}
            [1, _] => {
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "server security mode was modified during negotiation",
                ));
            }
            _ => {
                return Err(RnetError::new(
                    ErrorCode::AuthRejected,
                    "authentication rejected",
                ));
            }
        }
        shared
            .latencies
            .record(LatencyKind::CryptoHandshake, started.elapsed());
        mark_session_established(&shared, session)?;
        push_tcp_event(
            &shared,
            session_event(EventType::SessionOpened, endpoint, session),
        )
        .await;
        run_adaptive_session(
            Arc::clone(&shared),
            endpoint,
            session,
            stream,
            receiver,
            None,
            transport,
            SecurityController::new(Role::Client, initial_mode, INITIAL_EPOCH),
        )
        .await;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        fail_secure_session(&shared, endpoint, session, error);
    }
}

pub(crate) async fn write_protected(
    stream: &mut TcpStream,
    transport: &mut SecureTransport,
    epoch: u64,
    kind: ProtectedKind,
    payload: &[u8],
    max_payload: usize,
) -> Result<()> {
    let inner = encode_protected(&ProtectedMessage::new(kind, payload), max_payload)?;
    let ciphertext = transport.encrypt(&inner).map_err(map_security_error)?;
    write_wire(
        stream,
        &Record::new(RecordKind::Protected, epoch, &ciphertext),
        max_payload + 64,
    )
    .await
}

async fn read_protected(
    stream: &mut TcpStream,
    transport: &mut SecureTransport,
    max_payload: usize,
) -> Result<(u64, ProtectedMessage)> {
    let record = read_wire(stream, max_payload + 64).await?;
    let epoch = record.epoch;
    let message = decrypt_protected(transport, &record, max_payload)?;
    Ok((epoch, message))
}

pub(crate) fn decrypt_protected(
    transport: &mut SecureTransport,
    record: &Record,
    max_payload: usize,
) -> Result<ProtectedMessage> {
    if record.kind != RecordKind::Protected {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "expected protected record",
        ));
    }
    let plaintext = transport
        .decrypt(&record.payload)
        .map_err(map_security_error)?;
    decode_protected(&plaintext, max_payload)
}

fn expect_kind(record: Record, kind: RecordKind) -> Result<Record> {
    if record.kind != kind || record.epoch != 0 {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "unexpected handshake record",
        ));
    }
    Ok(record)
}

pub(crate) async fn read_wire(stream: &mut TcpStream, max_payload: usize) -> Result<Record> {
    let length = stream.read_u32().await? as usize;
    if length > max_payload + 20 {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "record is too large",
        ));
    }
    let mut encoded = vec![0; length];
    stream.read_exact(&mut encoded).await?;
    decode_record(&encoded, max_payload)
}

pub(crate) async fn write_wire(
    stream: &mut TcpStream,
    record: &Record,
    max_payload: usize,
) -> Result<()> {
    let encoded = encode_record(record, max_payload)?;
    let length = u32::try_from(encoded.len())
        .map_err(|_| RnetError::new(ErrorCode::MessageTooLarge, "record is too large"))?;
    stream.write_u32(length).await?;
    stream.write_all(&encoded).await?;
    Ok(())
}
