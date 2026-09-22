//! Game API for coalesced state snapshots staged ahead of transport send queues.

use crate::realtime::RealtimeQueueSnapshot;
use crate::runtime::GameRuntime;
use bytes::Bytes;
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use rnet_transport::LatestTransportSnapshot;

impl GameRuntime {
    /// Stages a replaceable snapshot. The key is scoped to the session; ordinary `send` remains
    /// reliable FIFO on TCP/KCP and never silently drops. A snapshot already handed to the
    /// transport worker or KCP send cache cannot be recalled.
    pub fn send_latest(&self, session: Handle, key: u64, payload: &[u8]) -> Result<()> {
        self.send_latest_with_tick(session, key, payload, None)
    }

    /// Adds an optional simulation tick to a replaceable snapshot.
    pub fn send_latest_with_tick(
        &self,
        session: Handle,
        key: u64,
        payload: &[u8],
        tick: Option<u32>,
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
                RnetError::new(ErrorCode::MessageTooLarge, "game snapshot length overflow")
            })?;
        self.network.validate_payload_len(session, envelope_len)?;
        // Closing a session removes its ready marker before clearing pending snapshots.
        // Keep that marker read-locked through insertion so a racing close cannot leave a
        // snapshot behind after its cleanup has completed.
        self.admission
            .with_ready(session, || {
                self.realtime
                    .lock()
                    .expect("realtime queue poisoned")
                    .enqueue(session, key, Bytes::copy_from_slice(payload), tick)
                    .map(|_| ())
            })
            .unwrap_or_else(|| {
                Err(RnetError::new(
                    ErrorCode::InvalidHandle,
                    "game session closed during snapshot admission",
                ))
            })
    }

    /// Non-blockingly forwards at most `capacity` staged snapshots in fair key order.
    /// A full transport queue rotates that snapshot behind other pending keys.
    pub fn flush_realtime(&self, capacity: usize) -> usize {
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        self.flush_realtime_inner(capacity)
    }

    pub(crate) fn flush_realtime_inner(&self, capacity: usize) -> usize {
        let attempts = capacity.min(
            self.realtime
                .lock()
                .expect("realtime queue poisoned")
                .snapshot()
                .queued_messages,
        );
        let mut forwarded = 0;
        for _ in 0..attempts {
            let message = self
                .realtime
                .lock()
                .expect("realtime queue poisoned")
                .pop_next();
            let Some(message) = message else { break };
            let result = self.send_game_latest_application(
                message.session,
                message.key,
                &message.payload,
                message.tick,
            );
            let mut queue = self.realtime.lock().expect("realtime queue poisoned");
            match result {
                Ok(()) => {
                    queue.record_forwarded();
                    forwarded += 1;
                }
                Err(error) if error.code() == ErrorCode::WouldBlock => queue.requeue(message),
                Err(_) => queue.record_send_failed(),
            }
        }
        forwarded
    }

    pub fn realtime_queue_snapshot(&self) -> RealtimeQueueSnapshot {
        self.realtime
            .lock()
            .expect("realtime queue poisoned")
            .snapshot()
    }

    /// Number of TCP/KCP snapshots replaced after leaving the game queue but before I/O pickup.
    pub fn transport_latest_replacements(&self) -> u64 {
        self.network.latest_transport_replacements()
    }

    /// Cumulative transport-pending replacement, worker-pickup and admission-failure telemetry.
    pub fn transport_latest_snapshot(&self) -> LatestTransportSnapshot {
        self.network.latest_transport_snapshot()
    }
}
