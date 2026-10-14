//! Game API for coalesced state snapshots staged ahead of transport send queues.

use crate::realtime::RealtimeQueueSnapshot;
use crate::runtime::GameRuntime;
use bytes::Bytes;
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use rnet_transport::LatestTransportSnapshot;

impl GameRuntime {
    /// Copies `payload` into replaceable staging for the game-ready `session`.
    /// `key` identifies a snapshot slot only within that session; zero is a valid key.
    /// Admission replaces any pending value for the same key, without notifying its receiver.
    /// The key is local scheduling metadata, not a business message type sent to the peer.
    ///
    /// TCP/KCP also coalesce pending transport slots until I/O worker pickup; UDP replacement
    /// ends at transport admission. Nothing recalls data already owned by socket/KCP buffers.
    /// Ordinary [`Self::send`] does not use replacement, but neither API promises delivery.
    /// Readiness, size, stopping and staging-budget errors follow the ordinary send contract.
    /// Keep polling or call [`Self::flush_realtime`] to forward snapshots.
    pub fn send_latest(&self, session: Handle, key: u64, payload: &[u8]) -> Result<()> {
        self.send_latest_with_tick(session, key, payload, None)
    }

    /// Adds an optional simulation tick to a replaceable snapshot.
    /// `session`, `key` and borrowed `payload` follow [`Self::send_latest`]. `tick` is forwarded
    /// unchanged; `None` omits it and `Some(0)` is valid. Replacing a value also replaces its tick.
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

    /// Attempts at most `capacity` staged snapshots in fair key order; zero performs no sends.
    /// A full transport queue rotates that snapshot behind other pending keys.
    /// Returns the number accepted by transport, not delivered. No network completion is waited
    /// for, but this method can wait for the shared poll lock; it does not drive heartbeat timers.
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

    /// Returns game-staging gauges and cumulative snapshot counters without resetting them.
    /// Transport-pending replacement is reported separately by [`Self::transport_latest_snapshot`].
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
