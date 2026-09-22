//! Bounded state owned by authenticated wire-v4 protocol negotiation.

use crate::config::GameProtocolRange;
use crate::event::{GameEvent, GameMessage};
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub(crate) const SELECT: u8 = 1;
pub(crate) const ACK: u8 = 2;
pub(crate) const READY: u8 = 3;
pub(crate) const CONTROL_LEN: usize = 37;
pub(crate) const RETRY_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const MAX_BUFFERED_MESSAGES: usize = 32;
pub(crate) const MAX_BUFFERED_BYTES: usize = 256 * 1024;

/// Low-cardinality wire-v4 early-data gauges and cumulative admission failures.
///
/// The limits are runtime-wide and independent of the transport event queue, even though they
/// inherit the same configured numeric ceilings. Peak and rejection counters remain cumulative
/// after runtime stop; current gauges return to zero.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RangeBufferSnapshot {
    pub buffered_messages: usize,
    pub buffered_bytes: usize,
    pub peak_buffered_messages: usize,
    pub peak_buffered_bytes: usize,
    pub max_buffered_messages: usize,
    pub max_buffered_bytes: usize,
    pub session_admission_rejected: u64,
    pub runtime_admission_rejected: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct ClientPending {
    pub(crate) protocol: GameProtocolRange,
    pub(crate) nonce: [u8; 16],
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Phase {
    ServerAwaitOpen,
    ServerAwaitAck,
    ServerReady,
    ClientAwaitSelect,
    ClientAwaitReady,
    ClientReady,
}

pub(crate) struct RangeSession {
    pub(crate) endpoint: Handle,
    pub(crate) protocol: GameProtocolRange,
    pub(crate) nonce: [u8; 16],
    pub(crate) selected: Option<u32>,
    pub(crate) server_handle: Handle,
    pub(crate) old_session: Option<Handle>,
    pub(crate) server_resume: bool,
    pub(crate) phase: Phase,
    pub(crate) deadline: Instant,
    pub(crate) next_retry: Instant,
    pub(crate) buffered: VecDeque<GameMessage>,
    pub(crate) buffered_bytes: usize,
}

impl RangeSession {
    pub(crate) fn control(&self, operation: u8) -> [u8; CONTROL_LEN] {
        let mut control = [0; CONTROL_LEN];
        control[0] = operation;
        control[1..17].copy_from_slice(&self.nonce);
        control[17..25].copy_from_slice(&self.protocol.protocol_id.to_be_bytes());
        control[25..29].copy_from_slice(&self.selected.unwrap_or_default().to_be_bytes());
        control[29..37].copy_from_slice(&self.server_handle.to_be_bytes());
        control
    }
}

struct CompletedEvent {
    event: GameEvent,
    /// `Some` means this message still owns one slot in the early-data budget. Empty payloads
    /// therefore remain distinguishable from ordinary unreserved public events.
    reserved_bytes: Option<usize>,
}

pub(crate) struct RangeRuntimeState {
    pub(crate) server_endpoints: HashMap<Handle, GameProtocolRange>,
    pub(crate) client_endpoints: HashMap<Handle, ClientPending>,
    pub(crate) sessions: HashMap<Handle, RangeSession>,
    completed: VecDeque<CompletedEvent>,
    max_buffered_messages: usize,
    max_buffered_bytes: usize,
    buffered_messages: usize,
    buffered_bytes: usize,
    peak_buffered_messages: usize,
    peak_buffered_bytes: usize,
    session_admission_rejected: u64,
    runtime_admission_rejected: u64,
}

impl RangeRuntimeState {
    pub(crate) fn with_buffer_limits(max_messages: usize, max_bytes: usize) -> Self {
        Self {
            server_endpoints: HashMap::new(),
            client_endpoints: HashMap::new(),
            sessions: HashMap::new(),
            completed: VecDeque::new(),
            max_buffered_messages: max_messages,
            max_buffered_bytes: max_bytes,
            buffered_messages: 0,
            buffered_bytes: 0,
            peak_buffered_messages: 0,
            peak_buffered_bytes: 0,
            session_admission_rejected: 0,
            runtime_admission_rejected: 0,
        }
    }

    /// Drops all endpoint/session state while retaining the configured runtime budget. A stopped
    /// runtime may be inspected again, so resetting must not silently replace finite limits with
    /// an unbounded default.
    pub(crate) fn clear(&mut self) {
        self.server_endpoints.clear();
        self.client_endpoints.clear();
        self.sessions.clear();
        self.completed.clear();
        self.buffered_messages = 0;
        self.buffered_bytes = 0;
    }

    pub(crate) fn forget_endpoint(&mut self, endpoint: Handle) {
        self.server_endpoints.remove(&endpoint);
        self.client_endpoints.remove(&endpoint);
    }

    pub(crate) fn forget_failed_client_endpoint(&mut self, endpoint: Handle) {
        self.client_endpoints.remove(&endpoint);
    }

    pub(crate) fn forget_session(&mut self, session: Handle) {
        if let Some(state) = self.sessions.remove(&session) {
            self.release_reservation(state.buffered.len(), state.buffered_bytes);
        }
        let mut released_messages = 0usize;
        let mut released_bytes = 0usize;
        self.completed.retain(|completed| {
            if !event_belongs_to_session(&completed.event, session) {
                return true;
            }
            if let Some(bytes) = completed.reserved_bytes {
                released_messages = released_messages.saturating_add(1);
                released_bytes = released_bytes.saturating_add(bytes);
            }
            false
        });
        self.release_reservation(released_messages, released_bytes);
    }

    pub(crate) fn drain_completed(&mut self, output: &mut Vec<GameEvent>, capacity: usize) {
        while output.len() < capacity {
            let Some(completed) = self.completed.pop_front() else {
                break;
            };
            self.release_completed(&completed);
            output.push(completed.event);
        }
    }

    pub(crate) fn queue_public(
        &mut self,
        event: GameEvent,
        output: &mut Vec<GameEvent>,
        capacity: usize,
    ) -> bool {
        let deferred = output.len() >= capacity || !self.completed.is_empty();
        if !deferred {
            output.push(event);
        } else {
            // Readiness or resume mapping must precede buffered business data.
            let insertion = match &event {
                GameEvent::SessionReady { session, .. }
                | GameEvent::SessionResumed {
                    new_session: session,
                    ..
                } => self.completed.iter().position(|pending| {
                    matches!(
                        &pending.event,
                        GameEvent::Message(message) if message.session == *session
                    )
                }),
                _ => None,
            };
            let completed = CompletedEvent {
                event,
                reserved_bytes: None,
            };
            if let Some(index) = insertion {
                self.completed.insert(index, completed);
            } else {
                self.completed.push_back(completed);
            }
        }
        self.drain_completed(output, capacity);
        deferred
    }

    /// Stages business data received before version negotiation is publicly ready. The limit is
    /// shared by every range session in the runtime, so many individually bounded peers cannot
    /// multiply retained memory beyond the configured application-event budget.
    pub(crate) fn buffer_message(&mut self, message: GameMessage) -> Result<Option<GameEvent>> {
        let Some(state) = self.sessions.get(&message.session) else {
            return Ok(Some(GameEvent::Message(message)));
        };
        if matches!(state.phase, Phase::ServerReady | Phase::ClientReady) {
            return Ok(Some(GameEvent::Message(message)));
        }
        let message_bytes = message.payload.len();
        let Some(session_bytes) = state.buffered_bytes.checked_add(message_bytes) else {
            self.session_admission_rejected = self.session_admission_rejected.saturating_add(1);
            return Err(protocol_error("v4 pending message budget overflow"));
        };
        if state.buffered.len() >= MAX_BUFFERED_MESSAGES || session_bytes > MAX_BUFFERED_BYTES {
            self.session_admission_rejected = self.session_admission_rejected.saturating_add(1);
            return Err(protocol_error(
                "v4 session pending-message budget exhausted",
            ));
        }
        let Some(runtime_messages) = self.buffered_messages.checked_add(1) else {
            self.runtime_admission_rejected = self.runtime_admission_rejected.saturating_add(1);
            return Err(protocol_error("v4 runtime pending-message count overflow"));
        };
        let Some(runtime_bytes) = self.buffered_bytes.checked_add(message_bytes) else {
            self.runtime_admission_rejected = self.runtime_admission_rejected.saturating_add(1);
            return Err(protocol_error("v4 runtime pending-message budget overflow"));
        };
        if runtime_messages > self.max_buffered_messages || runtime_bytes > self.max_buffered_bytes
        {
            self.runtime_admission_rejected = self.runtime_admission_rejected.saturating_add(1);
            return Err(protocol_error(
                "v4 runtime pending-message budget exhausted",
            ));
        }

        self.buffered_messages = runtime_messages;
        self.buffered_bytes = runtime_bytes;
        self.peak_buffered_messages = self.peak_buffered_messages.max(runtime_messages);
        self.peak_buffered_bytes = self.peak_buffered_bytes.max(runtime_bytes);
        let state = self
            .sessions
            .get_mut(&message.session)
            .expect("range session was validated under the same lock");
        state.buffered_bytes = session_bytes;
        state.buffered.push_back(message);
        Ok(None)
    }

    pub(crate) fn snapshot(&self) -> RangeBufferSnapshot {
        RangeBufferSnapshot {
            buffered_messages: self.buffered_messages,
            buffered_bytes: self.buffered_bytes,
            peak_buffered_messages: self.peak_buffered_messages,
            peak_buffered_bytes: self.peak_buffered_bytes,
            max_buffered_messages: self.max_buffered_messages,
            max_buffered_bytes: self.max_buffered_bytes,
            session_admission_rejected: self.session_admission_rejected,
            runtime_admission_rejected: self.runtime_admission_rejected,
        }
    }

    /// Moves early data behind the ready event without releasing its runtime reservation. Bytes
    /// remain charged until the application actually polls them or the session is forgotten.
    pub(crate) fn complete_buffered(&mut self, session: Handle) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        state.buffered_bytes = 0;
        let buffered = std::mem::take(&mut state.buffered);
        self.completed
            .extend(buffered.into_iter().map(|message| CompletedEvent {
                reserved_bytes: Some(message.payload.len()),
                event: GameEvent::Message(message),
            }));
    }

    #[cfg(test)]
    pub(crate) fn push_completed_for_test(&mut self, event: GameEvent) {
        self.completed.push_back(CompletedEvent {
            event,
            reserved_bytes: None,
        });
    }

    #[cfg(test)]
    pub(crate) fn completed_len_for_test(&self) -> usize {
        self.completed.len()
    }

    fn release_completed(&mut self, completed: &CompletedEvent) {
        if let Some(bytes) = completed.reserved_bytes {
            self.release_reservation(1, bytes);
        }
    }

    fn release_reservation(&mut self, messages: usize, bytes: usize) {
        debug_assert!(self.buffered_messages >= messages);
        debug_assert!(self.buffered_bytes >= bytes);
        self.buffered_messages = self.buffered_messages.saturating_sub(messages);
        self.buffered_bytes = self.buffered_bytes.saturating_sub(bytes);
    }

    /// Computes retry/expiry work without performing network I/O. Keeping this transition pure
    /// makes dropped-control behavior deterministic and prevents a busy public event queue from
    /// changing negotiation deadlines.
    pub(crate) fn due_controls(
        &mut self,
        now: Instant,
    ) -> (Vec<(Handle, [u8; CONTROL_LEN])>, Vec<Handle>) {
        let mut retries = Vec::new();
        let mut expired = Vec::new();
        for (&session, state) in &mut self.sessions {
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
    }
}

fn event_belongs_to_session(event: &GameEvent, session: Handle) -> bool {
    match event {
        GameEvent::Message(message) => message.session == session,
        GameEvent::SessionReady { session: ready, .. }
        | GameEvent::Writable { session: ready, .. }
        | GameEvent::SecurityChanged { session: ready, .. }
        | GameEvent::QualityChanged { session: ready, .. }
        | GameEvent::ResumeTicket { session: ready, .. } => *ready == session,
        GameEvent::SessionResumed { new_session, .. } => *new_session == session,
        _ => false,
    }
}

fn protocol_error(message: &'static str) -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn session(phase: Phase, now: Instant) -> RangeSession {
        RangeSession {
            endpoint: 1,
            protocol: GameProtocolRange::new(9, 2, 7),
            nonce: [3; 16],
            selected: Some(6),
            server_handle: 11,
            old_session: None,
            server_resume: false,
            phase,
            deadline: now + NEGOTIATION_TIMEOUT,
            next_retry: now,
            buffered: VecDeque::new(),
            buffered_bytes: 0,
        }
    }

    fn runtime_state() -> RangeRuntimeState {
        RangeRuntimeState::with_buffer_limits(4_096, 64 * 1024 * 1024)
    }

    #[test]
    fn dropped_select_and_ack_are_retried_at_a_bounded_interval() {
        let now = Instant::now();
        let mut state = runtime_state();
        state
            .sessions
            .insert(21, session(Phase::ServerAwaitAck, now));
        state
            .sessions
            .insert(22, session(Phase::ClientAwaitReady, now));

        let (mut retries, expired) = state.due_controls(now);
        retries.sort_by_key(|(session, _)| *session);
        assert!(expired.is_empty());
        assert_eq!(retries.len(), 2);
        assert_eq!(retries[0].0, 21);
        assert_eq!(retries[0].1[0], SELECT);
        assert_eq!(retries[1].0, 22);
        assert_eq!(retries[1].1[0], ACK);

        assert!(state.due_controls(now + RETRY_INTERVAL / 2).0.is_empty());
        assert_eq!(
            state.due_controls(now + RETRY_INTERVAL).0.len(),
            2,
            "each missing control is retried after one interval"
        );
    }

    #[test]
    fn negotiation_deadline_expires_waiters_without_retrying_ready_sessions() {
        let now = Instant::now();
        let mut state = runtime_state();
        for (handle, phase) in [
            (31, Phase::ServerAwaitAck),
            (32, Phase::ClientAwaitSelect),
            (33, Phase::ClientAwaitReady),
            (34, Phase::ServerReady),
            (35, Phase::ClientReady),
        ] {
            state.sessions.insert(handle, session(phase, now));
        }

        let (retries, mut expired) = state.due_controls(now + NEGOTIATION_TIMEOUT);
        expired.sort_unstable();
        assert!(retries.is_empty());
        assert_eq!(expired, vec![31, 32, 33]);
    }

    #[test]
    fn early_message_budget_is_runtime_wide_and_held_until_application_poll() {
        let now = Instant::now();
        let mut state = RangeRuntimeState::with_buffer_limits(1, 4);
        state
            .sessions
            .insert(41, session(Phase::ClientAwaitReady, now));
        state
            .sessions
            .insert(42, session(Phase::ClientAwaitReady, now));

        let message = |session, payload: &'static [u8]| GameMessage {
            endpoint: 1,
            session,
            sequence: None,
            tick: None,
            correlation_id: 0,
            payload: Bytes::from_static(payload),
        };
        assert!(state
            .buffer_message(message(41, b"1234"))
            .expect("first reservation")
            .is_none());
        assert_eq!(state.snapshot().buffered_messages, 1);
        assert_eq!(state.snapshot().buffered_bytes, 4);
        assert_eq!(
            state
                .buffer_message(message(42, b"x"))
                .expect_err("second session must share the runtime budget")
                .code(),
            rnet_core::ErrorCode::ProtocolError
        );

        state.complete_buffered(41);
        assert!(state.buffer_message(message(42, b"x")).is_err());
        assert_eq!(state.snapshot().runtime_admission_rejected, 2);

        let mut delivered = Vec::new();
        state.drain_completed(&mut delivered, 1);
        assert_eq!(delivered.len(), 1);
        assert_eq!(state.snapshot().buffered_messages, 0);
        assert_eq!(state.snapshot().buffered_bytes, 0);
        assert!(state
            .buffer_message(message(42, b"x"))
            .expect("poll releases the reservation")
            .is_none());
    }

    #[test]
    fn closing_negotiating_session_releases_its_early_message_budget() {
        let now = Instant::now();
        let mut state = RangeRuntimeState::with_buffer_limits(1, 4);
        state
            .sessions
            .insert(51, session(Phase::ServerAwaitAck, now));
        state
            .sessions
            .insert(52, session(Phase::ServerAwaitAck, now));
        state
            .sessions
            .insert(53, session(Phase::ServerAwaitAck, now));
        let message = |session| GameMessage {
            endpoint: 1,
            session,
            sequence: None,
            tick: None,
            correlation_id: 0,
            payload: Bytes::from_static(b"1234"),
        };

        assert!(state.buffer_message(message(51)).unwrap().is_none());
        state.forget_session(51);
        assert!(state
            .buffer_message(message(52))
            .expect("session close returns its reservation")
            .is_none());
        state.complete_buffered(52);
        state.forget_session(52);
        assert!(state
            .buffer_message(message(53))
            .expect("closing a completed-but-unpolled session returns its reservation")
            .is_none());
    }
}
