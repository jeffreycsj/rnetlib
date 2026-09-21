//! Bounded state owned by authenticated wire-v4 protocol negotiation.

use crate::config::GameProtocolRange;
use crate::event::{GameEvent, GameMessage};
use rnet_core::Handle;
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

#[derive(Default)]
pub(crate) struct RangeRuntimeState {
    pub(crate) server_endpoints: HashMap<Handle, GameProtocolRange>,
    pub(crate) client_endpoints: HashMap<Handle, ClientPending>,
    pub(crate) sessions: HashMap<Handle, RangeSession>,
    pub(crate) completed: VecDeque<GameEvent>,
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
                | GameEvent::SessionResumed {
                    new_session: session,
                    ..
                } => self.completed.iter().position(|pending| {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn dropped_select_and_ack_are_retried_at_a_bounded_interval() {
        let now = Instant::now();
        let mut state = RangeRuntimeState::default();
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
        let mut state = RangeRuntimeState::default();
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
}
