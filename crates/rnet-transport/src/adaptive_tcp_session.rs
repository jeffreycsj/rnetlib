//! Established adaptive TCP data path and security-transition loop.

use crate::adaptive_tcp::{decrypt_protected, write_protected, write_wire};
use crate::auto_rekey::AutoRekey;
use crate::event::{completed_operation, security_changed_event, SecurityOperation};
use crate::metrics::LatencyKind;
use crate::record_reader::RecordReader;
use crate::state::{
    game_control_event, message_event, push_tcp_event, receive_security_command,
    remove_session_with_reason, session_active, wait_for_deadline, Outbound, OutboundKind,
    SecurityCommand, Shared,
};
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use rnet_protocol::control::{
    decode_control, decode_record, encode_control, ProtectedKind, ProtectedMessage, Record,
    RecordKind, SecurityMode,
};
use rnet_protocol::decode_datagram;
use rnet_security::transition::{Effect, SecurityController};
use rnet_security::SecureTransport;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_adaptive_session(
    shared: Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    mut stream: TcpStream,
    mut receiver: mpsc::Receiver<Outbound>,
    mut commands: Option<mpsc::Receiver<SecurityCommand>>,
    mut transport: SecureTransport,
    mut controller: SecurityController,
) {
    let max_payload = shared.config.max_body_len + rnet_protocol::HEADER_LEN + 64;
    let mut records = RecordReader::new(max_payload + 84);
    let mut transition_deadline = None;
    let server_authoritative = commands.is_some();
    let mut automatic_rekey = AutoRekey::new(
        shared.config.security_policy.rekey_after,
        shared.config.security_policy.rekey_after_bytes,
        Instant::now(),
    );
    let reason = loop {
        // Drain complete records before polling again; an outbound wakeup cannot discard a
        // partially received length prefix or body from the previous read.
        match records.take() {
            Ok(Some(encoded)) => {
                let record = match decode_record(&encoded, max_payload + 64) {
                    Ok(record) => record,
                    Err(error) => break error.code(),
                };
                if let Err(error) = process_inbound(
                    &shared,
                    endpoint,
                    session,
                    &mut stream,
                    &mut transport,
                    &mut controller,
                    record,
                    max_payload,
                )
                .await
                {
                    shared
                        .metrics
                        .protocol_errors
                        .fetch_add(1, Ordering::Relaxed);
                    break error.code();
                }
                if !controller.is_transitioning() {
                    transition_deadline = None;
                }
                continue;
            }
            Ok(None) => {}
            Err(error) => break error.code(),
        }
        let automatic_deadline = if server_authoritative
            && controller.mode() == SecurityMode::Encrypted
            && !controller.is_transitioning()
        {
            let now = Instant::now();
            if automatic_rekey.is_due(now) {
                Some(tokio::time::Instant::now())
            } else {
                automatic_rekey
                    .next_deadline()
                    .map(tokio::time::Instant::from_std)
            }
        } else {
            None
        };
        tokio::select! {
            inbound = stream.read_buf(records.buffer_mut()) => {
                match inbound {
                    Ok(0) => break ErrorCode::IoError,
                    Ok(_) => {}
                    Err(_) => break ErrorCode::IoError,
                }
            }
            outbound = receiver.recv(), if !controller.is_transitioning() => {
                let Some(outbound) = outbound else { break ErrorCode::Cancelled };
                shared.latencies.record(LatencyKind::SendQueue, outbound.queued_at.elapsed());
                let result = match outbound.kind {
                    OutboundKind::GameControl => write_protected(
                        &mut stream, &mut transport, controller.epoch(), ProtectedKind::GameControl,
                        &outbound.bytes, max_payload,
                    ).await,
                    OutboundKind::Data if controller.mode() == SecurityMode::Encrypted => write_protected(
                        &mut stream, &mut transport, controller.epoch(), ProtectedKind::Data,
                        &outbound.bytes, max_payload,
                    ).await,
                    OutboundKind::Data => write_wire(
                        &mut stream,
                        &Record::new(RecordKind::PlainData, controller.epoch(), &outbound.bytes),
                        max_payload,
                    ).await,
                };
                if let Err(error) = result { break error.code(); }
                shared.metrics.bytes_sent.fetch_add(outbound.bytes.len() as u64, Ordering::Relaxed);
                if controller.mode() == SecurityMode::Encrypted || outbound.kind == OutboundKind::GameControl {
                    automatic_rekey.record_encrypted_bytes(outbound.bytes.len());
                }
            }
            command = receive_security_command(&mut commands) => {
                let Some(command) = command else { continue };
                let control = match command {
                    SecurityCommand::SetMode(mode) => controller.begin_switch(mode),
                    SecurityCommand::Rekey => {
                        automatic_rekey.mark_started(Instant::now());
                        controller.begin_rekey()
                    }
                };
                let Ok(control) = control else { continue };
                if write_control(
                    &mut stream, &mut transport, controller.epoch(), control, max_payload,
                ).await.is_err() { break ErrorCode::IoError; }
                transition_deadline = Some(
                    tokio::time::Instant::now() + shared.config.handshake_timeout,
                );
            }
            _ = wait_for_deadline(automatic_deadline), if automatic_deadline.is_some() => {
                let Ok(control) = controller.begin_rekey() else { continue };
                automatic_rekey.mark_started(Instant::now());
                if write_control(
                    &mut stream, &mut transport, controller.epoch(), control, max_payload,
                ).await.is_err() { break ErrorCode::IoError; }
                transition_deadline = Some(
                    tokio::time::Instant::now() + shared.config.handshake_timeout,
                );
            }
            _ = wait_for_deadline(transition_deadline), if transition_deadline.is_some() => {
                break ErrorCode::Timeout;
            },
        }
    };
    remove_session_with_reason(&shared, endpoint, session, reason);
}

#[allow(clippy::too_many_arguments)]
async fn process_inbound(
    shared: &Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    stream: &mut TcpStream,
    transport: &mut SecureTransport,
    controller: &mut SecurityController,
    record: Record,
    max_payload: usize,
) -> Result<()> {
    if !session_active(shared, session) || record.epoch != controller.epoch() {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "invalid security epoch",
        ));
    }
    let message = match record.kind {
        RecordKind::PlainData if controller.mode() == SecurityMode::Plaintext => {
            ProtectedMessage::new(ProtectedKind::Data, &record.payload)
        }
        RecordKind::Protected => decrypt_protected(transport, &record, max_payload)?,
        _ => {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "record mode mismatch",
            ))
        }
    };
    match message.kind {
        ProtectedKind::Data => {
            if record.kind == RecordKind::Protected && controller.mode() != SecurityMode::Encrypted
            {
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "encrypted data in plaintext mode",
                ));
            }
            let frame = decode_datagram(&message.payload, shared.config.max_body_len)?;
            shared
                .metrics
                .frames_received
                .fetch_add(1, Ordering::Relaxed);
            shared
                .metrics
                .bytes_received
                .fetch_add(frame.body.len() as u64, Ordering::Relaxed);
            push_tcp_event(shared, message_event(endpoint, session, frame)).await;
        }
        ProtectedKind::GameControl if record.kind == RecordKind::Protected => {
            if message.payload.len() > shared.config.max_body_len {
                return Err(RnetError::new(
                    ErrorCode::MessageTooLarge,
                    "game control exceeds the configured body limit",
                ));
            }
            push_tcp_event(
                shared,
                game_control_event(endpoint, session, message.payload),
            )
            .await;
        }
        ProtectedKind::GameControl => {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "game controls must be authenticated",
            ));
        }
        ProtectedKind::Control => {
            let control = decode_control(&message.payload)?;
            let operation = completed_operation(control.kind);
            let transition = controller.handle(control)?;
            apply_effect(transport, transition.before_response);
            if let Some(response) = transition.response {
                write_control(stream, transport, record.epoch, response, max_payload).await?;
            }
            apply_effect(transport, transition.after_response);
            if !controller.is_transitioning() {
                if let Some(operation) = operation {
                    push_security_changed(shared, endpoint, session, controller, operation).await;
                }
            }
        }
        ProtectedKind::AuthDecision | ProtectedKind::AuthAck => {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "unexpected auth decision",
            ));
        }
    }
    Ok(())
}

fn apply_effect(transport: &mut SecureTransport, effect: Effect) {
    if effect == Effect::Rekey {
        transport.rekey_incoming();
        transport.rekey_outgoing();
    }
}

async fn push_security_changed(
    shared: &Arc<Shared>,
    endpoint: Handle,
    session: Handle,
    controller: &SecurityController,
    operation: SecurityOperation,
) {
    let event = security_changed_event(
        endpoint,
        session,
        operation,
        controller.mode(),
        controller.epoch(),
    );
    push_tcp_event(shared, event).await;
}

async fn write_control(
    stream: &mut TcpStream,
    transport: &mut SecureTransport,
    epoch: u64,
    control: rnet_protocol::control::Control,
    max_payload: usize,
) -> Result<()> {
    write_protected(
        stream,
        transport,
        epoch,
        ProtectedKind::Control,
        &encode_control(control),
        max_payload,
    )
    .await
}
