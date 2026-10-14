//! Server-authoritative resume orchestration; never treats a ticket as application authorization.

use crate::envelope::{decode, encode_control, ControlKind, DecodedEnvelope};
use crate::event::{GameEvent, ResumeTicket};
use crate::resume::{ResumeClaim, ResumeRegistry, ResumeScope, SelectedProtocol, TICKET_LEN};
use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Event, Handle, Result, RnetError};
use rnet_transport::AuthRequest;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub(crate) struct ServerSessionMeta {
    pub(crate) peer_key: [u8; 32],
    pub(crate) protocol: SelectedProtocol,
}

pub(crate) struct ResumeRuntimeState {
    pub(crate) tickets: ResumeRegistry,
    pub(crate) server_sessions: HashMap<Handle, ServerSessionMeta>,
    pub(crate) pending_server: HashMap<Handle, ResumeClaim>,
    /// From claiming a ticket until publication of the new ready handle.
    pub(crate) inflight_server: HashMap<Handle, Handle>,
    pub(crate) revoked_inflight: HashSet<Handle>,
    pub(crate) client_endpoints: HashMap<Handle, Handle>,
    pub(crate) ttl: Duration,
}

impl ResumeRuntimeState {
    pub(crate) fn new(ttl: Duration, capacity: usize) -> Result<Self> {
        ResumeRegistry::validate_ttl(ttl)?;
        Ok(Self {
            tickets: ResumeRegistry::new(capacity)?,
            server_sessions: HashMap::new(),
            pending_server: HashMap::new(),
            inflight_server: HashMap::new(),
            revoked_inflight: HashSet::new(),
            client_endpoints: HashMap::new(),
            ttl,
        })
    }
}

impl GameRuntime {
    pub(crate) fn convert_game_auth_request(&self, event: &Event) -> Result<GameEvent> {
        let request = AuthRequest::from_event(event)?;
        Ok(self
            .range_auth_request(event, &request)
            .unwrap_or_else(|| self.reject_game_join(event)))
    }

    pub(crate) fn reject_game_join(&self, event: &Event) -> GameEvent {
        let _ = self.network.auth_decide(event.session, false);
        GameEvent::ProtocolRejected {
            endpoint: event.endpoint,
            session: event.session,
            reason: ErrorCode::ProtocolError,
        }
    }

    /// Server-only: issues a short-lived one-use ticket for an authorized logical player.
    /// `identity` is opaque to the library (1–64 bytes); do not put secrets in logs.
    /// `session` must be game-ready and server-side. The borrowed identity is copied into the
    /// runtime registry. Success queues protected control data; the client obtains the bytes in
    /// [`GameEvent::ResumeTicket`], not as this method's return value.
    /// Tickets expire, are one-use and bind the client key, endpoint and selected version.
    /// Restarting the server runtime invalidates them; reauthorization is always required.
    /// Invalid identity/role, full ticket storage, stale handles or control backpressure fail.
    pub fn issue_resume_ticket(&self, session: Handle, identity: &[u8]) -> Result<()> {
        self.ensure_game_ready(session)?;
        let endpoint = self
            .session_endpoints
            .lock()
            .expect("game session table poisoned")
            .get(&session)
            .copied()
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "game session closed"))?;
        let ticket = Zeroizing::new({
            let mut state = self.resume.lock().expect("resume state poisoned");
            let meta = state.server_sessions.get(&session).ok_or_else(|| {
                RnetError::new(
                    ErrorCode::NotSupported,
                    "only server sessions can issue tickets",
                )
            })?;
            let peer_key = meta.peer_key;
            let protocol = meta.protocol;
            let ttl = state.ttl;
            state.tickets.issue(
                ResumeScope {
                    endpoint,
                    session,
                    peer_key,
                    protocol,
                },
                identity,
                Instant::now(),
                ttl,
            )?
        });
        let send = encode_control(ControlKind::Resume, &ticket, self.maximum_envelope_len)
            .and_then(|envelope| self.network.send_game_control(session, &envelope));
        if send.is_err() {
            self.resume
                .lock()
                .expect("resume state poisoned")
                .tickets
                .revoke_ticket(&ticket);
        } else {
            self.resume_metrics
                .tickets_issued
                .fetch_add(1, Ordering::Relaxed);
        }
        send
    }

    pub(crate) fn handle_game_control_event(&self, event: &Event) -> Result<Option<GameEvent>> {
        match decode(&event.data, self.maximum_envelope_len)? {
            DecodedEnvelope::Control {
                kind: ControlKind::Heartbeat,
                ..
            } => self.handle_heartbeat_event(event),
            DecodedEnvelope::Control {
                kind: ControlKind::ClockSync,
                payload,
            } => self.handle_clock_sync_event(event, &payload),
            DecodedEnvelope::Control {
                kind: ControlKind::Resume,
                payload,
            } => {
                if payload.len() != TICKET_LEN
                    || self
                        .range
                        .lock()
                        .expect("range state poisoned")
                        .server_endpoints
                        .contains_key(&event.endpoint)
                {
                    return Err(RnetError::new(
                        ErrorCode::ProtocolError,
                        "invalid resume ticket control",
                    ));
                }
                if self.ensure_game_ready(event.session).is_err() {
                    return Ok(None);
                }
                Ok(Some(GameEvent::ResumeTicket {
                    endpoint: event.endpoint,
                    session: event.session,
                    ticket: ResumeTicket::new(payload.to_vec()),
                }))
            }
            DecodedEnvelope::Control {
                kind: ControlKind::Protocol,
                payload,
            } => self.handle_range_control(event, &payload),
            _ => Err(RnetError::new(
                ErrorCode::ProtocolError,
                "unsupported game control for this runtime version",
            )),
        }
    }

    pub(crate) fn forget_resume_session(&self, session: Handle) {
        let mut state = self.resume.lock().expect("resume state poisoned");
        state.server_sessions.remove(&session);
        state.pending_server.remove(&session);
        // A range handshake can close before READY while the old handle remains authoritative.
        // Do not retain an orphaned old-to-new mapping or its revocation marker indefinitely.
        let abandoned: Vec<_> = state
            .inflight_server
            .iter()
            .filter_map(|(old, new)| (*new == session).then_some(*old))
            .collect();
        for old in abandoned {
            state.inflight_server.remove(&old);
            state.revoked_inflight.remove(&old);
        }
    }

    pub(crate) fn revoke_resume_session(&self, session: Handle) {
        let pending = {
            let mut state = self.resume.lock().expect("resume state poisoned");
            state.tickets.revoke_session(session);
            let pending: Vec<_> = state
                .pending_server
                .iter()
                .filter_map(|(new_session, claim)| {
                    (claim.old_session == session).then_some(*new_session)
                })
                .collect();
            for new_session in &pending {
                state.pending_server.remove(new_session);
                state.server_sessions.remove(new_session);
            }
            let mut pending = pending;
            if let Some(new_session) = state.inflight_server.get(&session).copied() {
                state.revoked_inflight.insert(session);
                pending.push(new_session);
            }
            pending
        };
        // A ticket may already have been claimed while the game server decides to kick the
        // old identity. Close those pending routes before returning to the caller.
        for new_session in pending {
            self.resume_metrics
                .pending_revoked
                .fetch_add(1, Ordering::Relaxed);
            let _ = self
                .network
                .close_session(new_session, ErrorCode::Cancelled);
            self.forget_ready_session(new_session);
        }
    }
}
