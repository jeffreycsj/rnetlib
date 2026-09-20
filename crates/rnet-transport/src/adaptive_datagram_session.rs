//! Established-session receive path for adaptive UDP and KCP endpoints.

use crate::adaptive_codec::{apply, decrypt, protected_record, protocol};
use crate::adaptive_wire::DatagramWire;
use crate::event::{completed_operation, security_changed_event};
use crate::state::{
    game_control_event, message_event, session_active, try_push_session_event, Shared,
};
use rnet_core::{Handle, Result};
use rnet_protocol::control::{
    decode_control, encode_control, ControlKind, ProtectedKind, ProtectedMessage, Record,
    RecordKind, SecurityMode,
};
use rnet_protocol::decode_datagram;
use rnet_security::transition::SecurityController;
use rnet_security::DatagramTransport;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::net::UdpSocket;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_established(
    shared: &Arc<Shared>,
    endpoint: Handle,
    socket: &Arc<UdpSocket>,
    wire: &mut DatagramWire,
    peer: SocketAddr,
    session: Handle,
    transport: &mut DatagramTransport,
    controller: &mut SecurityController,
    record: Record,
) -> Result<()> {
    if !session_active(shared, session) || record.epoch != controller.epoch() {
        return protocol("invalid security epoch");
    }
    let message = match record.kind {
        RecordKind::PlainData if controller.mode() == SecurityMode::Plaintext => {
            ProtectedMessage::new(ProtectedKind::Data, &record.payload)
        }
        RecordKind::Protected => decrypt(transport, &record, shared.config.max_body_len + 64)?,
        _ => return protocol("record mode mismatch"),
    };
    match message.kind {
        ProtectedKind::Data => {
            if record.kind == RecordKind::Protected && controller.mode() != SecurityMode::Encrypted
            {
                return protocol("encrypted data received while plaintext mode is active");
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
            try_push_session_event(shared, message_event(endpoint, session, frame))?;
        }
        ProtectedKind::GameControl if record.kind == RecordKind::Protected => {
            if message.payload.len() > shared.config.max_body_len {
                return protocol("game control exceeds the configured body limit");
            }
            try_push_session_event(
                shared,
                game_control_event(endpoint, session, message.payload),
            )?;
        }
        ProtectedKind::GameControl => return protocol("game controls must be authenticated"),
        ProtectedKind::Control => {
            let control = decode_control(&message.payload)?;
            let operation = completed_operation(control.kind);
            let transition = controller.handle(control)?;
            apply(transport, transition.before_response);
            if let Some(response) = transition.response {
                let terminal = matches!(
                    response.kind,
                    ControlKind::SwitchAck | ControlKind::RekeyAck
                );
                let encoded = encode_control(response);
                if terminal {
                    send_protected_response(
                        socket,
                        wire,
                        peer,
                        transport,
                        record.epoch,
                        ProtectedKind::Control,
                        &encoded,
                        64,
                    )
                    .await?;
                } else {
                    send_protected(
                        socket,
                        wire,
                        peer,
                        transport,
                        record.epoch,
                        ProtectedKind::Control,
                        &encoded,
                        64,
                    )
                    .await?;
                }
            }
            apply(transport, transition.after_response);
            if !controller.is_transitioning() {
                if let Some(operation) = operation {
                    try_push_session_event(
                        shared,
                        security_changed_event(
                            endpoint,
                            session,
                            operation,
                            controller.mode(),
                            controller.epoch(),
                        ),
                    )?;
                }
            }
        }
        ProtectedKind::AuthAck if message.payload.is_empty() => {}
        _ => return protocol("unexpected protected message"),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_protected_response(
    socket: &Arc<UdpSocket>,
    wire: &mut DatagramWire,
    peer: SocketAddr,
    transport: &mut DatagramTransport,
    epoch: u64,
    kind: ProtectedKind,
    data: &[u8],
    max: usize,
) -> Result<()> {
    let record = protected_record(transport, epoch, kind, data, max)?;
    wire.send_response(socket, peer, &record, max + 64).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_protected(
    socket: &Arc<UdpSocket>,
    wire: &mut DatagramWire,
    peer: SocketAddr,
    transport: &mut DatagramTransport,
    epoch: u64,
    kind: ProtectedKind,
    data: &[u8],
    max: usize,
) -> Result<()> {
    let record = protected_record(transport, epoch, kind, data, max)?;
    wire.send_reliable(socket, peer, &record, max + 64).await
}
