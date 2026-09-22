//! Authenticated wire-v4 selection state machine and retry handling.

use crate::config::GameProtocol;
use crate::envelope::{encode_control, ControlKind};
use crate::event::{GameEvent, GameMessage};
use crate::join;
use crate::range_state::{
    Phase, RangeSession, ACK, CONTROL_LEN, NEGOTIATION_TIMEOUT, READY, RETRY_INTERVAL, SELECT,
};
use crate::resume_runtime::ServerSessionMeta;
use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Event, Handle, Result, RnetError};
use rnet_transport::AuthRequest;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::Instant;

impl GameRuntime {
    pub(crate) fn range_auth_request(
        &self,
        event: &Event,
        request: &AuthRequest<'_>,
    ) -> Option<GameEvent> {
        let protocol = self
            .range
            .lock()
            .expect("range state poisoned")
            .server_endpoints
            .get(&event.endpoint)
            .copied()?;
        let Ok(join) = join::decode_range(request.join_payload) else {
            return Some(self.reject_game_join(event));
        };
        let now = Instant::now();
        let (selected, resume_claim) = if let Some(ticket) = join.resume_ticket {
            self.resume_metrics
                .requests_received
                .fetch_add(1, Ordering::Relaxed);
            let mut state = self.resume.lock().expect("resume state poisoned");
            let pinned = state.tickets.pinned_version(
                ticket,
                event.endpoint,
                request.client_public_key,
                join.protocol.protocol_id,
                now,
            );
            let result = pinned.and_then(|version| {
                if version < join.protocol.min_version
                    || version > join.protocol.max_version
                    || version < protocol.min_version
                    || version > protocol.max_version
                {
                    return Err(protocol_error(
                        "resume version is outside the supported range",
                    ));
                }
                let exact = GameProtocol {
                    protocol_id: join.protocol.protocol_id,
                    version,
                    build_id: join.protocol.build_id,
                    capabilities: join.protocol.capabilities,
                };
                state
                    .tickets
                    .consume(
                        ticket,
                        event.endpoint,
                        request.client_public_key,
                        exact,
                        now,
                    )
                    .map(|claim| (version, claim))
            });
            match result {
                Ok((version, claim)) => (version, Some(claim)),
                Err(_) => {
                    self.resume_metrics
                        .tickets_rejected
                        .fetch_add(1, Ordering::Relaxed);
                    drop(state);
                    return Some(self.reject_game_join(event));
                }
            }
        } else {
            let Some(version) = join::select_version(join.protocol, protocol) else {
                return Some(self.reject_game_join(event));
            };
            (version, None)
        };
        self.range
            .lock()
            .expect("range state poisoned")
            .sessions
            .insert(
                event.session,
                RangeSession {
                    endpoint: event.endpoint,
                    protocol: join.protocol,
                    nonce: join.nonce,
                    selected: Some(selected),
                    server_handle: event.session,
                    old_session: None,
                    server_resume: false,
                    phase: Phase::ServerAwaitOpen,
                    deadline: now + NEGOTIATION_TIMEOUT,
                    next_retry: now,
                    buffered: VecDeque::new(),
                    buffered_bytes: 0,
                },
            );
        let mut resume = self.resume.lock().expect("resume state poisoned");
        resume.server_sessions.insert(
            event.session,
            ServerSessionMeta {
                peer_key: request.client_public_key,
                protocol: GameProtocol {
                    protocol_id: join.protocol.protocol_id,
                    version: selected,
                    build_id: join.protocol.build_id,
                    capabilities: join.protocol.capabilities,
                },
            },
        );
        if let Some(claim) = resume_claim {
            let old_session = claim.old_session;
            let identity = claim.identity.clone();
            resume.pending_server.insert(event.session, claim);
            Some(GameEvent::ResumeRequest {
                endpoint: event.endpoint,
                session: event.session,
                old_session,
                identity: identity.into(),
                client_public_key: request.client_public_key,
                join_ticket: join.ticket.to_vec().into(),
                build_id: join.protocol.build_id,
                capabilities: join.protocol.capabilities,
            })
        } else {
            Some(GameEvent::AuthRequest {
                endpoint: event.endpoint,
                session: event.session,
                client_public_key: request.client_public_key,
                join_ticket: join.ticket.to_vec().into(),
                build_id: join.protocol.build_id,
                capabilities: join.protocol.capabilities,
            })
        }
    }

    /// Moves a transport-opened range session into negotiation without publishing game readiness.
    pub(crate) fn start_range_session(
        &self,
        endpoint: Handle,
        session: Handle,
        old_session: Option<Handle>,
        server_resume: bool,
    ) -> Result<bool> {
        let now = Instant::now();
        let control = {
            let mut range = self.range.lock().expect("range state poisoned");
            if let Some(state) = range.sessions.get_mut(&session) {
                if state.phase != Phase::ServerAwaitOpen {
                    return Err(RnetError::new(
                        ErrorCode::ProtocolError,
                        "duplicate v4 session open",
                    ));
                }
                state.phase = Phase::ServerAwaitAck;
                state.old_session = old_session;
                state.server_resume = server_resume;
                state.deadline = now + NEGOTIATION_TIMEOUT;
                state.next_retry = now + RETRY_INTERVAL;
                Some(state.control(SELECT))
            } else if let Some(pending) = range.client_endpoints.remove(&endpoint) {
                range.sessions.insert(
                    session,
                    RangeSession {
                        endpoint,
                        protocol: pending.protocol,
                        nonce: pending.nonce,
                        selected: None,
                        server_handle: 0,
                        old_session,
                        server_resume: false,
                        phase: Phase::ClientAwaitSelect,
                        deadline: now + NEGOTIATION_TIMEOUT,
                        next_retry: now + RETRY_INTERVAL,
                        buffered: VecDeque::new(),
                        buffered_bytes: 0,
                    },
                );
                return Ok(true);
            } else {
                return Ok(false);
            }
        };
        if let Some(control) = control {
            if let Err(error) = self.send_range_control(session, &control) {
                if error.code() != ErrorCode::WouldBlock {
                    return Err(error);
                }
            }
        }
        Ok(true)
    }

    fn send_range_control(&self, session: Handle, payload: &[u8; CONTROL_LEN]) -> Result<()> {
        let envelope = encode_control(ControlKind::Protocol, payload, self.maximum_envelope_len)?;
        self.network.send_game_control(session, &envelope)
    }

    /// Only the protected control path calls this method. Plaintext application frames cannot
    /// impersonate selection, acknowledgment, or final readiness.
    pub(crate) fn handle_range_control(
        &self,
        event: &Event,
        payload: &[u8],
    ) -> Result<Option<GameEvent>> {
        if payload.len() != CONTROL_LEN {
            return Err(protocol_error("invalid protocol selection length"));
        }
        let operation = payload[0];
        let nonce: [u8; 16] = payload[1..17].try_into().expect("fixed nonce");
        let protocol_id = u64::from_be_bytes(payload[17..25].try_into().expect("fixed ID"));
        let version = u32::from_be_bytes(payload[25..29].try_into().expect("fixed version"));
        let server_handle = u64::from_be_bytes(payload[29..37].try_into().expect("fixed handle"));
        let mut range = self.range.lock().expect("range state poisoned");
        let state = range
            .sessions
            .get_mut(&event.session)
            .ok_or_else(|| protocol_error("protocol control on non-v4 session"))?;
        if state.endpoint != event.endpoint
            || state.nonce != nonce
            || state.protocol.protocol_id != protocol_id
            || version == 0
            || server_handle == 0
        {
            return Err(protocol_error("protocol selection does not match session"));
        }
        let now = Instant::now();
        if now >= state.deadline && !matches!(state.phase, Phase::ServerReady | Phase::ClientReady)
        {
            return Err(protocol_error("protocol selection expired"));
        }
        match (state.phase, operation) {
            (Phase::ClientAwaitSelect | Phase::ClientAwaitReady, SELECT) => {
                if version < state.protocol.min_version
                    || version > state.protocol.max_version
                    || (state.selected.is_some() && state.selected != Some(version))
                    || (state.server_handle != 0 && state.server_handle != server_handle)
                {
                    return Err(protocol_error(
                        "server selected an invalid protocol version",
                    ));
                }
                state.selected = Some(version);
                state.server_handle = server_handle;
                state.phase = Phase::ClientAwaitReady;
                state.next_retry = now + RETRY_INTERVAL;
                let control = state.control(ACK);
                drop(range);
                if let Err(error) = self.send_range_control(event.session, &control) {
                    if error.code() != ErrorCode::WouldBlock {
                        return Err(error);
                    }
                }
                Ok(None)
            }
            (Phase::ServerAwaitAck | Phase::ServerReady, ACK) => {
                if state.selected != Some(version) || state.server_handle != server_handle {
                    return Err(protocol_error(
                        "protocol acknowledgment does not match selection",
                    ));
                }
                let was_ready = state.phase == Phase::ServerReady;
                let control = state.control(READY);
                drop(range);
                if let Err(error) = self.send_range_control(event.session, &control) {
                    return if error.code() == ErrorCode::WouldBlock {
                        Ok(None)
                    } else {
                        Err(error)
                    };
                }
                if was_ready {
                    return Ok(None);
                }
                let mut range = self.range.lock().expect("range state poisoned");
                let Some(state) = range.sessions.get_mut(&event.session) else {
                    return Ok(None);
                };
                state.phase = Phase::ServerReady;
                let old_session = state.old_session;
                let server_resume = state.server_resume;
                range.complete_buffered(event.session);
                drop(range);
                self.publish_range_ready(event.endpoint, event.session, old_session, server_resume)
            }
            (Phase::ClientAwaitReady, READY) => {
                if state.selected != Some(version) || state.server_handle != server_handle {
                    return Err(protocol_error(
                        "protocol final confirmation does not match selection",
                    ));
                }
                state.phase = Phase::ClientReady;
                let old_session = state.old_session;
                range.complete_buffered(event.session);
                drop(range);
                self.publish_range_ready(event.endpoint, event.session, old_session, false)
            }
            (Phase::ClientReady, READY)
                if state.selected == Some(version) && state.server_handle == server_handle =>
            {
                Ok(None)
            }
            // A delayed retransmission can arrive after READY on an unordered datagram path.
            (Phase::ClientReady, SELECT)
                if state.selected == Some(version) && state.server_handle == server_handle =>
            {
                Ok(None)
            }
            _ => Err(protocol_error("unexpected protocol negotiation transition")),
        }
    }

    fn publish_range_ready(
        &self,
        endpoint: Handle,
        session: Handle,
        old_session: Option<Handle>,
        server_resume: bool,
    ) -> Result<Option<GameEvent>> {
        if self.network.validate_payload_len(session, 0).is_err() {
            self.forget_ready_session(session);
            return Ok(None);
        }
        if server_resume {
            let old = old_session
                .ok_or_else(|| protocol_error("resumed server session has no prior handle"))?;
            let mut resume = self.resume.lock().expect("resume state poisoned");
            if resume.revoked_inflight.remove(&old)
                || resume.inflight_server.get(&old) != Some(&session)
            {
                resume.inflight_server.remove(&old);
                drop(resume);
                let _ = self.network.close_session(session, ErrorCode::Cancelled);
                self.forget_ready_session(session);
                return Ok(None);
            }
            // The revocation check and publication share the resume lock, as in wire v3.
            self.admission.publish(session);
            resume.inflight_server.remove(&old);
            self.resume_metrics
                .sessions_resumed
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.admission.publish(session);
        }
        if let Some(old) = old_session {
            // Version negotiation is now complete. Invalidate the previous route immediately
            // before returning its replacement event; failed negotiation never reaches here.
            let _ = self.network.close_session(old, ErrorCode::Cancelled);
            self.forget_ready_session(old);
        }
        Ok(Some(if let Some(old_session) = old_session {
            GameEvent::SessionResumed {
                endpoint,
                old_session,
                new_session: session,
            }
        } else {
            GameEvent::SessionReady { endpoint, session }
        }))
    }

    pub(crate) fn buffer_range_message(&self, message: GameMessage) -> Result<Option<GameEvent>> {
        self.range
            .lock()
            .expect("range state poisoned")
            .buffer_message(message)
    }

    /// Retry controls independently of user-visible events so hidden controls cannot starve it.
    pub(crate) fn drive_range_negotiation(&self) {
        let now = Instant::now();
        let (retries, expired) = self
            .range
            .lock()
            .expect("range state poisoned")
            .due_controls(now);
        for (session, control) in retries {
            let _ = self.send_range_control(session, &control);
        }
        for session in expired {
            let _ = self.network.close_session(session, ErrorCode::Timeout);
            self.forget_ready_session(session);
        }
    }
}

fn protocol_error(message: &'static str) -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, message)
}

#[cfg(test)]
#[path = "range_runtime_tests.rs"]
mod tests;
