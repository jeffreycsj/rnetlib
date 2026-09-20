//! Game-facing events that hide compatibility framing and transport details.

use bytes::Bytes;
use rnet_core::{ErrorCode, Handle};
use rnet_transport::SecurityOperation;

/// Opaque business bytes received from an established game session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameMessage {
    pub endpoint: Handle,
    pub session: Handle,
    /// Network-owned sequence metadata, when requested by the advanced send API.
    pub sequence: Option<u32>,
    /// Simulation tick supplied by the sender, without any business-type interpretation.
    pub tick: Option<u32>,
    pub payload: Bytes,
}

/// Events emitted to game code. No variant exposes `msg_type`, `stream_id`, or transport routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameEvent {
    RuntimeStarted,
    EndpointOpened {
        endpoint: Handle,
    },
    EndpointError {
        endpoint: Handle,
        status: ErrorCode,
    },
    AuthRequest {
        endpoint: Handle,
        session: Handle,
        client_public_key: [u8; 32],
        join_ticket: Vec<u8>,
        /// Authenticated build metadata; application policy may reject it.
        build_id: u64,
        capabilities: u64,
    },
    /// The library rejected an incompatible or malformed game join before application auth.
    ProtocolRejected {
        endpoint: Handle,
        session: Handle,
        reason: ErrorCode,
    },
    SessionReady {
        endpoint: Handle,
        session: Handle,
    },
    SessionClosed {
        endpoint: Handle,
        session: Handle,
        reason: ErrorCode,
    },
    Message(GameMessage),
    Writable {
        endpoint: Handle,
        session: Handle,
    },
    JoinFailed {
        endpoint: Handle,
        session: Handle,
        reason: ErrorCode,
    },
    SecurityChanged {
        endpoint: Handle,
        session: Handle,
        encrypted: bool,
        epoch: u64,
        operation: SecurityOperation,
    },
    /// The peer sent framing that cannot be produced by this game API.
    ProtocolViolation {
        endpoint: Handle,
        session: Handle,
    },
    RuntimeStopped,
}
