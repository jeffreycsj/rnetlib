use crate::runtime::NetworkRuntime;
use crate::state::Outbound;
use crate::state::SecurityCommand;
use crate::state::SessionTarget;
use rnet_core::ErrorCode;
use rnet_core::Handle;
use rnet_core::Lifecycle;
use rnet_core::Result;
use rnet_core::RnetError;
use rnet_protocol::control::SecurityMode;
use rnet_protocol::encode_frame;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

/// Optional message metadata for request/response correlation.
///
/// Stream routing is intentionally owned by the transport. Most callers should use
/// [`NetworkRuntime::send`]; this type is only needed when a protocol requires a stable
/// correlation identifier across a request and its response.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SendOptions {
    pub correlation_id: u64,
}

impl NetworkRuntime {
    /// Requests a server-led data security mode change for an established unified session.
    pub fn set_security_mode(&self, session: Handle, mode: SecurityMode) -> Result<()> {
        if mode == SecurityMode::Plaintext
            && !self
                .shared
                .config
                .security_policy
                .allow_plaintext_business_data
        {
            return Err(RnetError::new(
                ErrorCode::NotSupported,
                "plaintext business data is disabled by security policy",
            ));
        }
        self.send_security_command(session, SecurityCommand::SetMode(mode))
    }

    /// Requests a server-led Noise rekey without changing the data security mode.
    pub fn rekey_session(&self, session: Handle) -> Result<()> {
        self.send_security_command(session, SecurityCommand::Rekey)
    }

    fn send_security_command(&self, session: Handle, command: SecurityCommand) -> Result<()> {
        let sender = self
            .shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .get(session)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?
            .security_commands
            .clone()
            .ok_or_else(|| {
                RnetError::new(
                    ErrorCode::InvalidState,
                    "session is not controlled by an adaptive server",
                )
            })?;
        sender.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => {
                RnetError::new(ErrorCode::WouldBlock, "security transition already queued")
            }
            mpsc::error::TrySendError::Closed(_) => {
                RnetError::new(ErrorCode::InvalidState, "session is closed")
            }
        })
    }

    pub fn auth_decide(&self, session: Handle, accept: bool) -> Result<()> {
        let sender = self
            .shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .get_mut(session)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?
            .auth_decision
            .take()
            .ok_or_else(|| {
                RnetError::new(
                    ErrorCode::InvalidState,
                    "session is not awaiting authentication",
                )
            })?;
        sender
            .send(accept)
            .map_err(|_| RnetError::new(ErrorCode::InvalidState, "authentication request expired"))
    }

    /// Enqueues opaque application bytes with all compatibility routing fields set to zero.
    ///
    /// This is the transport entry point used by the game facade. Business message types belong
    /// inside the caller's payload schema; TCP, UDP, or KCP routing was already fixed when the
    /// endpoint was created.
    pub fn send_payload(&self, session: Handle, payload: &[u8]) -> Result<()> {
        self.send_legacy(session, 0, 0, 0, payload)
    }

    /// Enqueues one application message. Transport stream selection is internal.
    ///
    /// The explicit message type is retained for source compatibility. New game-facing code uses
    /// [`NetworkRuntime::send_payload`] and keeps business typing inside its serialized payload.
    pub fn send(&self, session: Handle, msg_type: u32, payload: &[u8]) -> Result<()> {
        self.send_with_options(session, msg_type, payload, SendOptions::default())
    }

    /// Enqueues one application message with optional request/response correlation metadata.
    pub fn send_with_options(
        &self,
        session: Handle,
        msg_type: u32,
        payload: &[u8],
        options: SendOptions,
    ) -> Result<()> {
        self.send_legacy(session, msg_type, 0, options.correlation_id, payload)
    }

    /// Compatibility entry point for the stable C ABI. New Rust code should use `send` or
    /// `send_with_options`; stream identifiers are reserved for transport internals.
    #[doc(hidden)]
    pub fn send_legacy(
        &self,
        session: Handle,
        msg_type: u32,
        stream_id: u32,
        request_id: u64,
        payload: &[u8],
    ) -> Result<()> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting sends",
            ));
        }
        let frame = encode_frame(
            msg_type,
            stream_id,
            request_id,
            0,
            payload,
            self.shared.config.max_body_len,
        )?;
        let (target, session_budget) = {
            let sessions = self.shared.sessions.lock().expect("session table poisoned");
            let route = sessions
                .get(session)
                .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?;
            if !route.established {
                return Err(RnetError::new(
                    ErrorCode::HandshakeRequired,
                    "session handshake has not completed",
                ));
            }
            (route.target.clone(), route.queued_bytes.clone())
        };
        if let SessionTarget::Udp { max_frame_len, .. } = &target {
            if frame.len() > *max_frame_len {
                return Err(RnetError::new(
                    ErrorCode::MessageTooLarge,
                    "encoded frame exceeds UDP datagram limit",
                ));
            }
        }
        let outbound = Outbound::with_budgets(frame, &self.shared.send_budget, &session_budget)
            .inspect_err(|_| {
                self.shared
                    .metrics
                    .send_would_block
                    .fetch_add(1, Ordering::Relaxed);
            })?;
        let result = match target {
            SessionTarget::Tcp(sender) => sender.try_send(outbound),
            SessionTarget::Udp { sender, peer, .. } => {
                sender
                    .try_send((peer, outbound))
                    .map_err(|error| map_datagram_send_error(&self.shared, error, "UDP"))?;
                self.shared
                    .metrics
                    .frames_sent
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            SessionTarget::Kcp { sender, peer, .. } => {
                sender
                    .try_send((peer, outbound))
                    .map_err(|error| map_datagram_send_error(&self.shared, error, "KCP"))?;
                self.shared
                    .metrics
                    .frames_sent
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        };
        result.map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => {
                self.shared
                    .metrics
                    .send_would_block
                    .fetch_add(1, Ordering::Relaxed);
                RnetError::new(ErrorCode::WouldBlock, "session write queue is full")
            }
            mpsc::error::TrySendError::Closed(_) => {
                RnetError::new(ErrorCode::InvalidState, "session is closed")
            }
        })?;
        self.shared
            .metrics
            .frames_sent
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn map_datagram_send_error(
    shared: &crate::state::Shared,
    error: mpsc::error::TrySendError<(SocketAddr, Outbound)>,
    transport: &str,
) -> RnetError {
    match error {
        mpsc::error::TrySendError::Full(_) => {
            shared
                .metrics
                .send_would_block
                .fetch_add(1, Ordering::Relaxed);
            RnetError::new(
                ErrorCode::WouldBlock,
                format!("{transport} send queue is full"),
            )
        }
        mpsc::error::TrySendError::Closed(_) => RnetError::new(
            ErrorCode::InvalidState,
            format!("{transport} endpoint is closed"),
        ),
    }
}
