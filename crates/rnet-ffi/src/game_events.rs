//! Converts typed game events without leaking transport framing into the C interface.

use crate::game_abi::*;
use crate::game_registry::GameBuffers;
use rnet_game::GameEvent;

pub(crate) fn encode(event: GameEvent, buffers: &GameBuffers) -> RnetGameEvent {
    let mut out = RnetGameEvent::default();
    match event {
        GameEvent::RuntimeStarted => out.event_type = RNET_GAME_RUNTIME_STARTED,
        GameEvent::EndpointOpened { endpoint } => {
            out.event_type = RNET_GAME_ENDPOINT_OPENED;
            out.endpoint = endpoint;
        }
        GameEvent::EndpointError { endpoint, status } => {
            out.event_type = RNET_GAME_ENDPOINT_ERROR;
            out.endpoint = endpoint;
            out.status = status as i32;
        }
        GameEvent::AuthRequest {
            endpoint,
            session,
            client_public_key,
            join_ticket,
            build_id,
            capabilities,
        } => {
            out.event_type = RNET_GAME_AUTH_REQUEST;
            out.endpoint = endpoint;
            out.session = session;
            out.client_public_key = client_public_key;
            out.build_id = build_id;
            out.capabilities = capabilities;
            put_data(&mut out, buffers, join_ticket.as_bytes().to_vec());
        }
        GameEvent::ResumeRequest {
            endpoint,
            session,
            old_session,
            identity,
            client_public_key,
            join_ticket,
            build_id,
            capabilities,
        } => {
            out.event_type = RNET_GAME_RESUME_REQUEST;
            out.endpoint = endpoint;
            out.session = session;
            out.related_session = old_session;
            out.client_public_key = client_public_key;
            out.build_id = build_id;
            out.capabilities = capabilities;
            put_data(&mut out, buffers, identity.as_bytes().to_vec());
            if let Some(view) = buffers.insert(join_ticket.as_bytes().to_vec()) {
                out.aux_data = view.ptr;
                out.aux_data_len = view.len;
                out.aux_buffer_token = view.token;
            }
        }
        GameEvent::ProtocolRejected {
            endpoint,
            session,
            reason,
        } => {
            out.event_type = RNET_GAME_PROTOCOL_REJECTED;
            out.endpoint = endpoint;
            out.session = session;
            out.status = reason as i32;
        }
        GameEvent::SessionReady { endpoint, session } => {
            out.event_type = RNET_GAME_SESSION_READY;
            out.endpoint = endpoint;
            out.session = session;
        }
        GameEvent::SessionResumed {
            endpoint,
            old_session,
            new_session,
        } => {
            out.event_type = RNET_GAME_SESSION_RESUMED;
            out.endpoint = endpoint;
            out.session = new_session;
            out.related_session = old_session;
        }
        GameEvent::ResumeTicket {
            endpoint,
            session,
            ticket,
        } => {
            out.event_type = RNET_GAME_RESUME_TICKET;
            out.endpoint = endpoint;
            out.session = session;
            put_data(&mut out, buffers, ticket.as_bytes().to_vec());
        }
        GameEvent::SessionClosed {
            endpoint,
            session,
            reason,
        } => {
            out.event_type = RNET_GAME_SESSION_CLOSED;
            out.endpoint = endpoint;
            out.session = session;
            out.status = reason as i32;
        }
        GameEvent::Message(message) => {
            out.event_type = RNET_GAME_MESSAGE;
            out.endpoint = message.endpoint;
            out.session = message.session;
            if let Some(sequence) = message.sequence {
                out.has_sequence = 1;
                out.sequence = sequence;
            }
            if let Some(tick) = message.tick {
                out.has_tick = 1;
                out.tick = tick;
            }
            put_data(&mut out, buffers, message.payload.to_vec());
        }
        GameEvent::Writable { endpoint, session } => {
            out.event_type = RNET_GAME_WRITABLE;
            out.endpoint = endpoint;
            out.session = session;
        }
        GameEvent::JoinFailed {
            endpoint,
            session,
            reason,
        } => {
            out.event_type = RNET_GAME_JOIN_FAILED;
            out.endpoint = endpoint;
            out.session = session;
            out.status = reason as i32;
        }
        GameEvent::SecurityChanged {
            endpoint,
            session,
            encrypted,
            epoch,
            operation,
        } => {
            out.event_type = RNET_GAME_SECURITY_CHANGED;
            out.endpoint = endpoint;
            out.session = session;
            out.encrypted = u32::from(encrypted);
            out.security_epoch = epoch;
            out.security_operation = operation as u32;
        }
        GameEvent::QualityChanged {
            endpoint,
            session,
            quality,
        } => {
            out.event_type = RNET_GAME_QUALITY_CHANGED;
            out.endpoint = endpoint;
            out.session = session;
            out.quality_grade = quality_grade(quality.grade);
            out.quality_basis = quality_basis(quality.basis);
            out.last_rtt_us = micros(quality.last_rtt);
            out.jitter_us = micros(quality.jitter);
            out.quality_samples = quality.samples;
        }
        GameEvent::ProtocolViolation { endpoint, session } => {
            out.event_type = RNET_GAME_PROTOCOL_VIOLATION;
            out.endpoint = endpoint;
            out.session = session;
        }
        GameEvent::RuntimeStopped => out.event_type = RNET_GAME_RUNTIME_STOPPED,
    }
    out
}

fn put_data(out: &mut RnetGameEvent, buffers: &GameBuffers, data: Vec<u8>) {
    if let Some(view) = buffers.insert(data) {
        out.data = view.ptr;
        out.data_len = view.len;
        out.buffer_token = view.token;
    }
}
