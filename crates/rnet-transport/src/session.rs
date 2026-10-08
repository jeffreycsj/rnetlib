use crate::runtime::NetworkRuntime;
use crate::state::Outbound;
use crate::state::OutboundKind;
use crate::state::SecurityCommand;
use crate::state::SessionTarget;
use bytes::Bytes;
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
    /// Checks size and ready-state without allocating or enqueuing a frame.
    /// Staging layers use the same UDP frame ceiling as the normal send path.
    pub fn validate_payload_len(&self, session: Handle, payload_len: usize) -> Result<()> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting sends",
            ));
        }
        if payload_len > self.shared.config.max_body_len {
            return Err(RnetError::new(
                ErrorCode::MessageTooLarge,
                "payload exceeds configured body limit",
            ));
        }
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
        if let SessionTarget::Udp { max_frame_len, .. } = &route.target {
            let frame_len = payload_len
                .checked_add(rnet_protocol::HEADER_LEN)
                .ok_or_else(|| {
                    RnetError::new(ErrorCode::MessageTooLarge, "frame length overflow")
                })?;
            if frame_len > *max_frame_len {
                return Err(RnetError::new(
                    ErrorCode::MessageTooLarge,
                    "encoded frame exceeds UDP datagram limit",
                ));
            }
        }
        Ok(())
    }

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

    /// Enqueues opaque application bytes on the session's fixed TCP, UDP, or KCP transport.
    /// Success means accepted by the local bounded queue, not delivered to the peer.
    /// An unfinished handshake, closed session, oversized payload, or full queue returns an error.
    ///
    /// Business message types belong in the payload; typed and legacy sends are not supported.
    ///
    /// ```compile_fail,E0061
    /// # fn old_send(runtime: &rnet_transport::NetworkRuntime) {
    /// runtime.send(1, 7, b"payload").unwrap();
    /// # }
    /// ```
    ///
    /// ```compile_fail,E0061
    /// # fn old_options(runtime: &rnet_transport::NetworkRuntime) {
    /// runtime.send_with_options(1, 7, b"payload", Default::default()).unwrap();
    /// # }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// # fn old_legacy(runtime: &rnet_transport::NetworkRuntime) {
    /// runtime.send_legacy(1, 7, 2, 99, b"payload").unwrap();
    /// # }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// # fn old_alias(runtime: &rnet_transport::NetworkRuntime) {
    /// runtime.send_payload(1, b"payload").unwrap();
    /// # }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// # fn old_options_alias(runtime: &rnet_transport::NetworkRuntime) {
    /// runtime.send_payload_with_options(1, b"payload", Default::default()).unwrap();
    /// # }
    /// ```
    pub fn send(&self, session: Handle, payload: &[u8]) -> Result<()> {
        self.send_with_options(session, payload, SendOptions::default())
    }

    /// Enqueues one application message with optional request/response correlation metadata.
    pub fn send_with_options(
        &self,
        session: Handle,
        payload: &[u8],
        options: SendOptions,
    ) -> Result<()> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting sends",
            ));
        }
        let frame = encode_frame(
            0,
            0,
            options.correlation_id,
            0,
            payload,
            self.shared.config.max_body_len,
        )?;
        self.enqueue_encoded(session, frame, OutboundKind::Data)
    }

    /// Sends a game-library control through Noise even when business data is plaintext.
    /// The game facade owns the control envelope; ordinary applications should use `send`.
    #[doc(hidden)]
    pub fn send_game_control(&self, session: Handle, payload: &[u8]) -> Result<()> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting sends",
            ));
        }
        if payload.len() > self.shared.config.max_body_len {
            return Err(RnetError::new(
                ErrorCode::MessageTooLarge,
                "game control exceeds the configured body limit",
            ));
        }
        self.enqueue_encoded(
            session,
            Bytes::copy_from_slice(payload),
            OutboundKind::GameControl,
        )
    }

    fn enqueue_encoded(&self, session: Handle, frame: Bytes, kind: OutboundKind) -> Result<()> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting sends",
            ));
        }
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
            if kind == OutboundKind::GameControl && !route.allows_game_controls {
                return Err(RnetError::new(
                    ErrorCode::NotSupported,
                    "game controls require an adaptive Noise session",
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
        let outbound = match kind {
            OutboundKind::Data => {
                Outbound::with_budgets(frame, &self.shared.send_budget, &session_budget)
            }
            OutboundKind::GameControl => {
                Outbound::with_game_control(frame, &self.shared.send_budget, &session_budget)
            }
        }
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
