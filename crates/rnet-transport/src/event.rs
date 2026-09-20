//! Typed views over event payloads exposed by the transport runtime.

use rnet_core::{ErrorCode, Event, EventType, Handle, Result, RnetError};
use rnet_protocol::control::{ControlKind, SecurityMode};

/// The server-authoritative security operation that completed for a session.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityOperation {
    ModeSwitch = 1,
    Rekey = 2,
}

/// Decoded `SecurityChanged` event data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecurityChange {
    pub operation: SecurityOperation,
    pub mode: SecurityMode,
    pub epoch: u64,
}

impl SecurityChange {
    pub fn from_event(event: &Event) -> Result<Self> {
        if event.event_type != EventType::SecurityChanged || event.data.len() != 10 {
            return invalid_event("event is not a valid security change");
        }
        let mode = SecurityMode::try_from(event.data[0])?;
        let epoch = u64::from_be_bytes(
            event.data[1..9]
                .try_into()
                .expect("validated security event length"),
        );
        let operation = match event.data[9] {
            1 => SecurityOperation::ModeSwitch,
            2 => SecurityOperation::Rekey,
            _ => return invalid_event("security event has an unknown operation"),
        };
        Ok(Self {
            operation,
            mode,
            epoch,
        })
    }
}

/// Decoded authentication request with an owned key and borrowed join payload.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuthRequest<'a> {
    pub client_public_key: [u8; 32],
    pub join_payload: &'a [u8],
}

impl std::fmt::Debug for AuthRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthRequest")
            .field("client_public_key", &self.client_public_key)
            .field("join_payload_len", &self.join_payload.len())
            .finish()
    }
}

impl<'a> AuthRequest<'a> {
    pub fn from_event(event: &'a Event) -> Result<Self> {
        if event.event_type != EventType::AuthRequest || event.data.len() < 32 {
            return invalid_event("event is not a valid authentication request");
        }
        Ok(Self {
            client_public_key: event.data[..32]
                .try_into()
                .expect("validated authentication event length"),
            join_payload: &event.data[32..],
        })
    }
}

pub(crate) fn security_changed_event(
    endpoint: Handle,
    session: Handle,
    operation: SecurityOperation,
    mode: SecurityMode,
    epoch: u64,
) -> Event {
    let mut event = Event::simple(EventType::SecurityChanged);
    event.endpoint = endpoint;
    event.session = session;
    event.data.push(mode as u8);
    event.data.extend_from_slice(&epoch.to_be_bytes());
    event.data.push(operation as u8);
    event
}

pub(crate) fn completed_operation(kind: ControlKind) -> Option<SecurityOperation> {
    match kind {
        ControlKind::SwitchCommit | ControlKind::SwitchAck => Some(SecurityOperation::ModeSwitch),
        ControlKind::RekeyCommit | ControlKind::RekeyAck => Some(SecurityOperation::Rekey),
        _ => None,
    }
}

fn invalid_event<T>(message: &str) -> Result<T> {
    Err(RnetError::new(ErrorCode::InvalidArgument, message))
}
