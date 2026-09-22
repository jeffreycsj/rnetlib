//! Runtime integration for bounded advanced-send scheduling.

use crate::runtime::{GameRuntime, GameSendOptions};
use crate::scheduler::{ScheduledMessage, ScheduledQueueSnapshot};
use bytes::Bytes;
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use std::time::Instant;

impl GameRuntime {
    pub(crate) fn send_scheduled(
        &self,
        session: Handle,
        payload: &[u8],
        options: GameSendOptions,
    ) -> Result<()> {
        if self.admission.is_stopping() {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "game runtime is stopping",
            ));
        }
        self.ensure_game_ready(session)?;
        let udp = self
            .udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .contains_key(&session);
        let envelope_len = payload
            .len()
            .checked_add(if udp { 16 } else { 12 })
            .ok_or_else(|| {
                RnetError::new(ErrorCode::MessageTooLarge, "game message length overflow")
            })?;
        self.network.validate_payload_len(session, envelope_len)?;
        let expires_at = options
            .expires_after
            .map(|duration| {
                Instant::now().checked_add(duration).ok_or_else(|| {
                    RnetError::new(ErrorCode::InvalidArgument, "game send expiry overflows")
                })
            })
            .transpose()?;
        // Keep readiness stable through admission. Session cleanup takes the write side before
        // removing queued messages, so it cannot miss an enqueue that already passed validation.
        self.admission
            .with_ready(session, || {
                self.scheduled
                    .lock()
                    .expect("scheduled queue poisoned")
                    .enqueue(ScheduledMessage {
                        session,
                        payload: Bytes::copy_from_slice(payload),
                        sequence: options.sequence,
                        tick: options.tick,
                        correlation_id: options.correlation_id,
                        priority: options.priority,
                        enqueued_at: Instant::now(),
                        expires_at,
                    })
            })
            .unwrap_or_else(|| {
                Err(RnetError::new(
                    ErrorCode::InvalidHandle,
                    "game session closed during scheduled-send admission",
                ))
            })
    }

    /// Attempts a bounded number of priority-aware sends without blocking the game loop.
    pub fn flush_scheduled(&self, capacity: usize) -> usize {
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        self.flush_scheduled_inner(capacity)
    }

    pub(crate) fn flush_scheduled_inner(&self, capacity: usize) -> usize {
        let attempts = capacity.min(self.scheduled_queue_snapshot().queued_messages);
        let mut forwarded = 0;
        for _ in 0..attempts {
            let message = self
                .scheduled
                .lock()
                .expect("scheduled queue poisoned")
                .pop_next(Instant::now());
            let Some(message) = message else { break };
            let result = self.send_game_application(
                message.session,
                &message.payload,
                GameSendOptions {
                    sequence: message.sequence,
                    tick: message.tick,
                    correlation_id: message.correlation_id,
                    priority: message.priority,
                    expires_after: None,
                },
            );
            let mut queue = self.scheduled.lock().expect("scheduled queue poisoned");
            match result {
                Ok(()) => {
                    queue.record_forwarded(&message);
                    forwarded += 1;
                }
                Err(error) if error.code() == ErrorCode::WouldBlock => queue.requeue(message),
                Err(error) => {
                    queue.record_send_failed(&message);
                    drop(queue);
                    self.log_scheduled_send_failure(
                        message.session,
                        error.code(),
                        message.correlation_id,
                    );
                }
            }
        }
        forwarded
    }

    pub fn scheduled_queue_snapshot(&self) -> ScheduledQueueSnapshot {
        self.scheduled
            .lock()
            .expect("scheduled queue poisoned")
            .snapshot()
    }
}
