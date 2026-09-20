//! Per-peer handshake and established state for adaptive datagram endpoints.

use crate::state::SecurityCommand;
use rnet_core::Handle;
use rnet_protocol::control::SecurityMode;
use rnet_security::transition::SecurityController;
use rnet_security::{DatagramTransport, InitiatorHandshake, ResponderHandshake};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

pub(crate) const INITIAL_EPOCH: u64 = 1;

pub(crate) enum Peer {
    Cookie {
        expires_at: Instant,
    },
    ServerFirst {
        handshake: ResponderHandshake,
        started: Instant,
    },
    ServerFinish {
        handshake: ResponderHandshake,
        started: Instant,
    },
    ServerAuth {
        session: Handle,
        transport: DatagramTransport,
        auth: oneshot::Receiver<bool>,
        commands: mpsc::Receiver<SecurityCommand>,
        mode: SecurityMode,
        auth_started: Instant,
    },
    ClientHello {
        session: Handle,
        started: Instant,
    },
    ClientResponse {
        session: Handle,
        handshake: InitiatorHandshake,
        join: Vec<u8>,
        mode: SecurityMode,
        started: Instant,
    },
    ClientAuth {
        session: Handle,
        transport: DatagramTransport,
        mode: SecurityMode,
        auth_started: Instant,
        connect_started: Instant,
    },
    Established {
        session: Handle,
        transport: DatagramTransport,
        controller: SecurityController,
        commands: Option<mpsc::Receiver<SecurityCommand>>,
        /// Last successfully received or queued wire activity for idle reclamation.
        last_activity: Instant,
        /// Deadline for the current server-led security barrier, if any.
        transition_deadline: Option<Instant>,
    },
}

impl Peer {
    pub(crate) fn session(&self) -> Option<Handle> {
        match self {
            Self::ServerAuth { session, .. }
            | Self::ClientHello { session, .. }
            | Self::ClientResponse { session, .. }
            | Self::ClientAuth { session, .. }
            | Self::Established { session, .. } => Some(*session),
            Self::Cookie { .. } | Self::ServerFirst { .. } | Self::ServerFinish { .. } => None,
        }
    }

    pub(crate) fn is_established(&self) -> bool {
        matches!(self, Self::Established { .. })
    }
}
