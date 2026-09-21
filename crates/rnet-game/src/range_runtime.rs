//! Explicit wire-v4 version negotiation over authenticated game controls.

use crate::config::{
    GameClientConfig, GameHostClientConfig, GameProtocol, GameProtocolRange, GameRangeClientConfig,
    GameRangeHostClientConfig, GameRangeServerConfig,
};
use crate::envelope::{encode_control, ControlKind};
use crate::event::{GameEvent, GameMessage};
use crate::join;
use crate::resume::TICKET_LEN;
use crate::resume_runtime::ServerSessionMeta;
use crate::runtime::GameRuntime;
use ring::rand::{SecureRandom, SystemRandom};
use rnet_core::{ErrorCode, Event, Handle, Result, RnetError};
use rnet_protocol::control::SecurityMode;
use rnet_transport::{AuthRequest, ServerConfig};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const SELECT: u8 = 1;
const ACK: u8 = 2;
const READY: u8 = 3;
const CONTROL_LEN: usize = 37;
const RETRY_INTERVAL: Duration = Duration::from_millis(100);
const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BUFFERED_MESSAGES: usize = 32;
const MAX_BUFFERED_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct ClientPending {
    protocol: GameProtocolRange,
    nonce: [u8; 16],
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    ServerAwaitOpen,
    ServerAwaitAck,
    ServerReady,
    ClientAwaitSelect,
    ClientAwaitReady,
    ClientReady,
}

struct RangeSession {
    endpoint: Handle,
    protocol: GameProtocolRange,
    nonce: [u8; 16],
    selected: Option<u32>,
    server_handle: Handle,
    old_session: Option<Handle>,
    server_resume: bool,
    phase: Phase,
    deadline: Instant,
    next_retry: Instant,
    buffered: VecDeque<GameMessage>,
    buffered_bytes: usize,
}

impl RangeSession {
    fn control(&self, operation: u8) -> [u8; CONTROL_LEN] {
        let mut control = [0; CONTROL_LEN];
        control[0] = operation;
        control[1..17].copy_from_slice(&self.nonce);
        control[17..25].copy_from_slice(&self.protocol.protocol_id.to_be_bytes());
        control[25..29].copy_from_slice(&self.selected.unwrap_or_default().to_be_bytes());
        control[29..37].copy_from_slice(&self.server_handle.to_be_bytes());
        control
    }
}

#[derive(Default)]
pub(crate) struct RangeRuntimeState {
    pub(crate) server_endpoints: HashMap<Handle, GameProtocolRange>,
    client_endpoints: HashMap<Handle, ClientPending>,
    sessions: HashMap<Handle, RangeSession>,
    completed: VecDeque<GameEvent>,
}

impl RangeRuntimeState {
    pub(crate) fn forget_endpoint(&mut self, endpoint: Handle) {
        self.server_endpoints.remove(&endpoint);
        self.client_endpoints.remove(&endpoint);
    }

    pub(crate) fn forget_failed_client_endpoint(&mut self, endpoint: Handle) {
        self.client_endpoints.remove(&endpoint);
    }

    pub(crate) fn forget_session(&mut self, session: Handle) {
        self.sessions.remove(&session);
        self.completed.retain(|event| match event {
            GameEvent::Message(message) => message.session != session,
            GameEvent::SessionReady { session: ready, .. }
            | GameEvent::Writable { session: ready, .. }
            | GameEvent::SecurityChanged { session: ready, .. }
            | GameEvent::QualityChanged { session: ready, .. }
            | GameEvent::ResumeTicket { session: ready, .. } => *ready != session,
            GameEvent::SessionResumed { new_session, .. } => *new_session != session,
            _ => true,
        });
    }

    pub(crate) fn drain_completed(&mut self, output: &mut Vec<GameEvent>, capacity: usize) {
        while output.len() < capacity {
            let Some(event) = self.completed.pop_front() else {
                break;
            };
            output.push(event);
        }
    }

    pub(crate) fn queue_public(
        &mut self,
        event: GameEvent,
        output: &mut Vec<GameEvent>,
        capacity: usize,
    ) {
        if output.len() < capacity && self.completed.is_empty() {
            output.push(event);
        } else {
            // Readiness or resume mapping must precede buffered business data.
            let insertion = match &event {
                GameEvent::SessionReady { session, .. }
                | GameEvent::SessionResumed { new_session: session, .. } => self.completed.iter().position(|pending| {
                    matches!(pending, GameEvent::Message(message) if message.session == *session)
                }),
                _ => None,
            };
            if let Some(index) = insertion {
                self.completed.insert(index, event);
            } else {
                self.completed.push_back(event);
            }
        }
        self.drain_completed(output, capacity);
    }
}

fn nonce() -> Result<[u8; 16]> {
    let mut nonce = [0; 16];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| RnetError::new(ErrorCode::CryptoError, "protocol nonce RNG failed"))?;
    if nonce == [0; 16] {
        return Err(RnetError::new(
            ErrorCode::CryptoError,
            "protocol nonce RNG returned zero",
        ));
    }
    Ok(nonce)
}

impl GameRuntime {
    /// Starts an explicit wire-v4 range listener; exact-version listeners retain wire v3.
    pub fn listen_range(&self, config: GameRangeServerConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.network.listen(ServerConfig {
            transport: config.transport,
            bind_addr: config.bind_addr,
            local_key: config.local_key,
            initial_security: if config.initial_encryption {
                SecurityMode::Encrypted
            } else {
                SecurityMode::Plaintext
            },
        })?;
        self.range
            .lock()
            .expect("range state poisoned")
            .server_endpoints
            .insert(endpoint, config.protocol);
        self.endpoint_transports
            .lock()
            .expect("game endpoint table poisoned")
            .insert(endpoint, config.transport);
        Ok(endpoint)
    }

    /// Joins a range listener using a fresh nonce and a distinct wire-v4 marker.
    pub fn connect_range(&self, config: GameRangeClientConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range(
            config.protocol,
            nonce,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_prepared(
            GameClientConfig {
                transport: config.transport,
                bind_addr: config.bind_addr,
                remote_addr: config.remote_addr,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            None,
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Hostname variant of `connect_range` with the same authenticated version selection.
    pub fn connect_host_range(&self, config: GameRangeHostClientConfig) -> Result<Handle> {
        if !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid protocol range",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range(
            config.protocol,
            nonce,
            &config.join_ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_host_prepared(
            GameHostClientConfig {
                transport: config.transport,
                host: config.host,
                port: config.port,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            None,
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Resumes within this runtime; the ticket pins the original selected version.
    pub fn connect_range_resume(
        &self,
        config: GameRangeClientConfig,
        old_session: Handle,
        ticket: &[u8],
    ) -> Result<Handle> {
        if old_session == 0 || ticket.len() != TICKET_LEN || !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid range resume request",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range_resume(
            config.protocol,
            nonce,
            &config.join_ticket,
            ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_prepared(
            GameClientConfig {
                transport: config.transport,
                bind_addr: config.bind_addr,
                remote_addr: config.remote_addr,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            Some(old_session),
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Hostname variant of single-runtime wire-v4 resume.
    pub fn connect_host_range_resume(
        &self,
        config: GameRangeHostClientConfig,
        old_session: Handle,
        ticket: &[u8],
    ) -> Result<Handle> {
        if old_session == 0 || ticket.len() != TICKET_LEN || !config.protocol.is_valid() {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid range resume request",
            ));
        }
        let nonce = nonce()?;
        let payload = join::encode_range_resume(
            config.protocol,
            nonce,
            &config.join_ticket,
            ticket,
            self.maximum_envelope_len.min(60 * 1024),
        )?;
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let endpoint = self.connect_host_prepared(
            GameHostClientConfig {
                transport: config.transport,
                host: config.host,
                port: config.port,
                join_ticket: config.join_ticket,
                protocol: GameProtocol::new(
                    config.protocol.protocol_id,
                    config.protocol.max_version,
                ),
            },
            payload,
            Some(old_session),
        )?;
        self.range
            .lock()
            .expect("range state poisoned")
            .client_endpoints
            .insert(
                endpoint,
                ClientPending {
                    protocol: config.protocol,
                    nonce,
                },
            );
        Ok(endpoint)
    }

    /// Returns the negotiated version for a live wire-v4 session, including pending server auth.
    pub fn selected_protocol_version(&self, session: Handle) -> Result<u32> {
        self.range
            .lock()
            .expect("range state poisoned")
            .sessions
            .get(&session)
            .and_then(|state| state.selected)
            .ok_or_else(|| {
                RnetError::new(
                    ErrorCode::InvalidHandle,
                    "no selected range version for session",
                )
            })
    }

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
                let buffered = std::mem::take(&mut state.buffered);
                range
                    .completed
                    .extend(buffered.into_iter().map(GameEvent::Message));
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
                let buffered = std::mem::take(&mut state.buffered);
                range
                    .completed
                    .extend(buffered.into_iter().map(GameEvent::Message));
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
            self.ready_sessions
                .write()
                .expect("game ready table poisoned")
                .insert(session);
            resume.inflight_server.remove(&old);
            self.resume_metrics
                .sessions_resumed
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.ready_sessions
                .write()
                .expect("game ready table poisoned")
                .insert(session);
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
        let mut range = self.range.lock().expect("range state poisoned");
        let Some(state) = range.sessions.get_mut(&message.session) else {
            return Ok(Some(GameEvent::Message(message)));
        };
        if matches!(state.phase, Phase::ServerReady | Phase::ClientReady) {
            return Ok(Some(GameEvent::Message(message)));
        }
        let next = state
            .buffered_bytes
            .checked_add(message.payload.len())
            .ok_or_else(|| protocol_error("v4 pending message budget overflow"))?;
        if state.buffered.len() >= MAX_BUFFERED_MESSAGES || next > MAX_BUFFERED_BYTES {
            return Err(protocol_error("v4 pending message budget exhausted"));
        }
        state.buffered_bytes = next;
        state.buffered.push_back(message);
        Ok(None)
    }

    /// Retry controls independently of user-visible events so hidden controls cannot starve it.
    pub(crate) fn drive_range_negotiation(&self) {
        let now = Instant::now();
        let (retries, expired) = {
            let mut range = self.range.lock().expect("range state poisoned");
            let mut retries = Vec::new();
            let mut expired = Vec::new();
            for (&session, state) in &mut range.sessions {
                let operation = match state.phase {
                    Phase::ServerAwaitAck => SELECT,
                    Phase::ClientAwaitReady => ACK,
                    Phase::ClientAwaitSelect => {
                        if now >= state.deadline {
                            expired.push(session);
                        }
                        continue;
                    }
                    Phase::ServerAwaitOpen | Phase::ServerReady | Phase::ClientReady => continue,
                };
                if now >= state.deadline {
                    expired.push(session);
                } else if now >= state.next_retry {
                    state.next_retry = now + RETRY_INTERVAL;
                    retries.push((session, state.control(operation)));
                }
            }
            (retries, expired)
        };
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
mod tests {
    use super::*;
    use crate::config::GameRuntimeConfig;
    use bytes::Bytes;
    use rnet_core::EventType;

    #[test]
    fn late_duplicate_select_does_not_disconnect_a_ready_udp_client() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
        let now = Instant::now();
        let session = RangeSession {
            endpoint: 31,
            protocol: GameProtocolRange::new(55, 2, 5),
            nonce: [7; 16],
            selected: Some(4),
            server_handle: 99,
            old_session: None,
            server_resume: false,
            phase: Phase::ClientReady,
            deadline: now + Duration::from_secs(1),
            next_retry: now,
            buffered: VecDeque::new(),
            buffered_bytes: 0,
        };
        let control = session.control(SELECT);
        runtime.range.lock().unwrap().sessions.insert(32, session);
        let mut event = Event::simple(EventType::GameControl);
        event.endpoint = 31;
        event.session = 32;
        assert!(runtime
            .handle_range_control(&event, &control)
            .unwrap()
            .is_none());
    }

    #[test]
    fn completed_early_messages_respect_public_poll_capacity() {
        let mut state = RangeRuntimeState::default();
        for byte in [1, 2] {
            state.completed.push_back(GameEvent::Message(GameMessage {
                endpoint: 1,
                session: 2,
                sequence: None,
                tick: None,
                payload: Bytes::from(vec![byte]),
            }));
        }
        let mut first = Vec::new();
        state.drain_completed(&mut first, 1);
        assert_eq!(first.len(), 1);
        let mut second = Vec::new();
        state.drain_completed(&mut second, 1);
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn resumed_mapping_precedes_buffered_business_data() {
        let mut state = RangeRuntimeState::default();
        state.completed.push_back(GameEvent::Message(GameMessage {
            endpoint: 1,
            session: 3,
            sequence: None,
            tick: None,
            payload: Bytes::from_static(b"early"),
        }));
        let mut output = Vec::new();
        state.queue_public(
            GameEvent::SessionResumed {
                endpoint: 1,
                old_session: 2,
                new_session: 3,
            },
            &mut output,
            1,
        );
        assert!(matches!(
            output.as_slice(),
            [GameEvent::SessionResumed { new_session: 3, .. }]
        ));
        let mut next = Vec::new();
        state.drain_completed(&mut next, 1);
        assert!(matches!(next.as_slice(), [GameEvent::Message(message)] if message.session == 3));
    }

    #[test]
    fn closing_range_session_discards_deferred_ready_and_message_events() {
        let mut state = RangeRuntimeState::default();
        state.completed.push_back(GameEvent::SessionReady {
            endpoint: 1,
            session: 9,
        });
        state.completed.push_back(GameEvent::Message(GameMessage {
            endpoint: 1,
            session: 9,
            sequence: None,
            tick: None,
            payload: Bytes::from_static(b"late"),
        }));
        state.forget_session(9);
        let mut output = Vec::new();
        state.drain_completed(&mut output, 10);
        assert!(
            output.is_empty(),
            "closed sessions cannot publish stale deferred events"
        );
    }

    #[test]
    fn stopping_runtime_clears_v4_listener_policy() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
        let listener = runtime
            .listen_range(GameRangeServerConfig {
                transport: rnet_core::Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: rnet_security::Keypair::generate().unwrap(),
                initial_encryption: true,
                protocol: GameProtocolRange::new(3, 1, 2),
            })
            .unwrap();
        assert!(runtime
            .range
            .lock()
            .unwrap()
            .server_endpoints
            .contains_key(&listener));
        runtime.stop(Duration::ZERO).unwrap();
        assert!(runtime.range.lock().unwrap().server_endpoints.is_empty());
    }

    #[test]
    fn failed_join_does_not_remove_v4_listener_policy() {
        let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
        let listener = runtime
            .listen_range(GameRangeServerConfig {
                transport: rnet_core::Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: rnet_security::Keypair::generate().unwrap(),
                initial_encryption: true,
                protocol: GameProtocolRange::new(4, 1, 3),
            })
            .unwrap();
        let mut failed = Event::simple(EventType::JoinFailed);
        failed.endpoint = listener;
        failed.session = 123;
        runtime.convert_event(failed).unwrap();
        assert!(runtime
            .range
            .lock()
            .unwrap()
            .server_endpoints
            .contains_key(&listener));
    }
}
