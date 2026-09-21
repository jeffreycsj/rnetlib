//! Keyed, replaceable transport-pending data slots. A worker's `take` is the recall boundary.

use crate::runtime::NetworkRuntime;
use crate::state::{ByteBudget, Outbound, SessionTarget};
use bytes::Bytes;
use rnet_core::{ErrorCode, Handle, Lifecycle, Result, RnetError};
use rnet_protocol::encode_frame;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;

pub(crate) struct LatestSlot {
    current: Mutex<Option<Outbound>>,
    worker_pickups: Arc<AtomicU64>,
}

impl LatestSlot {
    pub(crate) fn new(outbound: Outbound, worker_pickups: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(Some(outbound)),
            worker_pickups,
        })
    }

    /// Returns false once the worker has taken the slot and its message is no longer recallable.
    pub(crate) fn replace(&self, bytes: Bytes) -> Result<bool> {
        let mut current = self.current.lock().expect("latest slot poisoned");
        let Some(outbound) = current.as_mut() else {
            return Ok(false);
        };
        outbound.replace_bytes(bytes)?;
        Ok(true)
    }

    pub(crate) fn take(&self) -> Option<Outbound> {
        let outbound = self.current.lock().expect("latest slot poisoned").take();
        if outbound.is_some() {
            self.worker_pickups.fetch_add(1, Ordering::Relaxed);
        }
        outbound
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.current.lock().expect("latest slot poisoned").is_some()
    }
}

/// Admission result for a keyed snapshot. `Replaced` means the old frame never left the
/// transport-pending slot; it does not imply cancellation after the I/O worker took that slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatestSendOutcome {
    Queued,
    Replaced,
}

/// Cumulative, low-cardinality telemetry for replaceable transport-pending snapshots.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LatestTransportSnapshot {
    pub pending_replaced: u64,
    /// Slots taken by an I/O worker. From this boundary onward the payload cannot be recalled.
    pub worker_pickups: u64,
    pub admission_would_block: u64,
    pub admission_invalid_handle: u64,
    pub admission_invalid_state: u64,
    pub admission_handshake_required: u64,
    pub admission_not_supported: u64,
    pub admission_message_too_large: u64,
    pub admission_other_failures: u64,
}

#[derive(Default)]
pub(crate) struct LatestRegistry {
    slots: Mutex<HashMap<Handle, HashMap<u64, Weak<LatestSlot>>>>,
    replaced: AtomicU64,
    worker_pickups: Arc<AtomicU64>,
    admission_would_block: AtomicU64,
    admission_invalid_handle: AtomicU64,
    admission_invalid_state: AtomicU64,
    admission_handshake_required: AtomicU64,
    admission_not_supported: AtomicU64,
    admission_message_too_large: AtomicU64,
    admission_other_failures: AtomicU64,
}

impl LatestRegistry {
    pub(crate) fn enqueue(
        &self,
        session: Handle,
        key: u64,
        frame: Bytes,
        target: &SessionTarget,
        runtime_budget: &Arc<ByteBudget>,
        session_budget: &Arc<ByteBudget>,
    ) -> Result<LatestSendOutcome> {
        let mut slots = self.slots.lock().expect("latest registry poisoned");
        let session_slots = slots.entry(session).or_default();
        // Stale weak entries are removed on each admission for this session. This bounds map
        // cardinality by live pending keys, not all keys ever used by a long-lived player.
        session_slots.retain(|_, weak| weak.upgrade().is_some_and(|slot| slot.is_pending()));
        if let Some(slot) = session_slots.get(&key).and_then(Weak::upgrade) {
            if slot.replace(frame.clone())? {
                self.replaced.fetch_add(1, Ordering::Relaxed);
                return Ok(LatestSendOutcome::Replaced);
            }
        }
        let outbound = Outbound::with_budgets(frame, runtime_budget, session_budget)?;
        let slot = LatestSlot::new(outbound, Arc::clone(&self.worker_pickups));
        let pending = Outbound::with_latest_slot(Arc::clone(&slot));
        match target {
            SessionTarget::Tcp(sender) => sender.try_send(pending).map_err(map_tcp_send_error)?,
            SessionTarget::Kcp { sender, peer, .. } => sender
                .try_send((*peer, pending))
                .map_err(map_kcp_send_error)?,
            SessionTarget::Udp { .. } => {
                return Err(RnetError::new(
                    ErrorCode::NotSupported,
                    "UDP uses its own datagram latest path",
                ))
            }
        }
        session_slots.insert(key, Arc::downgrade(&slot));
        Ok(LatestSendOutcome::Queued)
    }

    pub(crate) fn forget_session(&self, session: Handle) {
        self.slots
            .lock()
            .expect("latest registry poisoned")
            .remove(&session);
    }

    pub(crate) fn replacements(&self) -> u64 {
        self.replaced.load(Ordering::Relaxed)
    }

    pub(crate) fn record_admission_failure(&self, code: ErrorCode) {
        let counter = match code {
            ErrorCode::WouldBlock => &self.admission_would_block,
            ErrorCode::InvalidHandle => &self.admission_invalid_handle,
            ErrorCode::InvalidState => &self.admission_invalid_state,
            ErrorCode::HandshakeRequired => &self.admission_handshake_required,
            ErrorCode::NotSupported => &self.admission_not_supported,
            ErrorCode::MessageTooLarge => &self.admission_message_too_large,
            _ => &self.admission_other_failures,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> LatestTransportSnapshot {
        LatestTransportSnapshot {
            pending_replaced: self.replaced.load(Ordering::Relaxed),
            worker_pickups: self.worker_pickups.load(Ordering::Relaxed),
            admission_would_block: self.admission_would_block.load(Ordering::Relaxed),
            admission_invalid_handle: self.admission_invalid_handle.load(Ordering::Relaxed),
            admission_invalid_state: self.admission_invalid_state.load(Ordering::Relaxed),
            admission_handshake_required: self.admission_handshake_required.load(Ordering::Relaxed),
            admission_not_supported: self.admission_not_supported.load(Ordering::Relaxed),
            admission_message_too_large: self.admission_message_too_large.load(Ordering::Relaxed),
            admission_other_failures: self.admission_other_failures.load(Ordering::Relaxed),
        }
    }
}

fn map_tcp_send_error(error: mpsc::error::TrySendError<Outbound>) -> RnetError {
    match error {
        mpsc::error::TrySendError::Full(_) => {
            RnetError::new(ErrorCode::WouldBlock, "session latest queue is full")
        }
        mpsc::error::TrySendError::Closed(_) => {
            RnetError::new(ErrorCode::InvalidState, "session latest queue is closed")
        }
    }
}

fn map_kcp_send_error(
    error: mpsc::error::TrySendError<(std::net::SocketAddr, Outbound)>,
) -> RnetError {
    match error {
        mpsc::error::TrySendError::Full(_) => {
            RnetError::new(ErrorCode::WouldBlock, "KCP latest queue is full")
        }
        mpsc::error::TrySendError::Closed(_) => {
            RnetError::new(ErrorCode::InvalidState, "KCP latest queue is closed")
        }
    }
}

impl NetworkRuntime {
    /// Enqueues one game snapshot with a per-session key while preserving the normal send API.
    pub fn send_payload_latest(
        &self,
        session: Handle,
        key: u64,
        payload: &[u8],
    ) -> Result<LatestSendOutcome> {
        let result = self.send_payload_latest_inner(session, key, payload);
        if let Err(error) = &result {
            self.shared.latest.record_admission_failure(error.code());
        }
        result
    }

    fn send_payload_latest_inner(
        &self,
        session: Handle,
        key: u64,
        payload: &[u8],
    ) -> Result<LatestSendOutcome> {
        if self.shared.state.load() != Lifecycle::Running {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime is not accepting sends",
            ));
        }
        let frame = encode_frame(0, 0, 0, 0, payload, self.shared.config.max_body_len)?;
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
            if !route.allows_game_controls {
                return Err(RnetError::new(
                    ErrorCode::NotSupported,
                    "latest transport slots require an adaptive game session",
                ));
            }
            (route.target.clone(), Arc::clone(&route.queued_bytes))
        };
        let outcome = self
            .shared
            .latest
            .enqueue(
                session,
                key,
                frame,
                &target,
                &self.shared.send_budget,
                &session_budget,
            )
            .inspect_err(|error| {
                if error.code() == ErrorCode::WouldBlock {
                    self.shared
                        .metrics
                        .send_would_block
                        .fetch_add(1, Ordering::Relaxed);
                }
            })?;
        if outcome == LatestSendOutcome::Queued {
            self.shared
                .metrics
                .frames_sent
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(outcome)
    }

    pub fn latest_transport_replacements(&self) -> u64 {
        self.shared.latest.replacements()
    }

    pub fn latest_transport_snapshot(&self) -> LatestTransportSnapshot {
        self.shared.latest.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use crate::latest::{LatestRegistry, LatestSendOutcome, LatestSlot};
    use crate::state::{ByteBudget, Outbound, SessionTarget};
    use bytes::Bytes;
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Barrier};

    #[test]
    fn replacing_pending_snapshot_releases_byte_budget_and_delivers_only_new_bytes() {
        let runtime = ByteBudget::new(64);
        let session = ByteBudget::new(64);
        let first = Outbound::with_budgets(Bytes::from_static(b"old-snapshot"), &runtime, &session)
            .unwrap();
        let pickups = Arc::new(AtomicU64::new(0));
        let slot = LatestSlot::new(first, Arc::clone(&pickups));
        assert_eq!(runtime.used(), 12);
        slot.replace(Bytes::from_static(b"new")).unwrap();
        assert_eq!(runtime.used(), 3);
        assert_eq!(session.used(), 3);
        let pending = Outbound::with_latest_slot(Arc::clone(&slot));
        let delivered = pending.resolve_latest().unwrap();
        assert_eq!(delivered.bytes, Bytes::from_static(b"new"));
        assert!(slot.take().is_none());
        assert_eq!(pickups.load(std::sync::atomic::Ordering::Relaxed), 1);
        drop(delivered);
        assert_eq!(runtime.used(), 0);
        assert_eq!(session.used(), 0);
    }

    #[test]
    fn same_key_replaces_one_pending_tcp_queue_entry_without_reordering_other_keys() {
        let registry = LatestRegistry::default();
        let runtime = ByteBudget::new(64);
        let session = ByteBudget::new(64);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        let target = SessionTarget::Tcp(sender);
        assert_eq!(
            registry
                .enqueue(
                    7,
                    1,
                    Bytes::from_static(b"old"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap(),
            LatestSendOutcome::Queued
        );
        assert_eq!(
            registry
                .enqueue(
                    7,
                    2,
                    Bytes::from_static(b"middle"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap(),
            LatestSendOutcome::Queued
        );
        assert_eq!(
            registry
                .enqueue(
                    7,
                    1,
                    Bytes::from_static(b"new"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap(),
            LatestSendOutcome::Replaced
        );
        assert_eq!(runtime.used(), 9);
        assert_eq!(
            receiver.try_recv().unwrap().resolve_latest().unwrap().bytes,
            Bytes::from_static(b"new")
        );
        assert_eq!(
            receiver.try_recv().unwrap().resolve_latest().unwrap().bytes,
            Bytes::from_static(b"middle")
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(runtime.used(), 0);
    }

    #[test]
    fn kcp_pending_slot_replaces_before_worker_take_but_not_after() {
        let registry = LatestRegistry::default();
        let runtime = ByteBudget::new(128);
        let session = ByteBudget::new(128);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        let target = SessionTarget::Kcp {
            sender,
            peer: "127.0.0.1:9999".parse().unwrap(),
            cleanup: None,
        };
        assert_eq!(
            registry
                .enqueue(
                    7,
                    4,
                    Bytes::from_static(b"first"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap(),
            LatestSendOutcome::Queued
        );
        assert_eq!(
            registry
                .enqueue(
                    7,
                    4,
                    Bytes::from_static(b"second"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap(),
            LatestSendOutcome::Replaced
        );
        let first = receiver.try_recv().unwrap().1.resolve_latest().unwrap();
        assert_eq!(first.bytes, Bytes::from_static(b"second"));
        assert_eq!(
            registry
                .enqueue(
                    7,
                    4,
                    Bytes::from_static(b"third"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap(),
            LatestSendOutcome::Queued
        );
        let third = receiver.try_recv().unwrap().1.resolve_latest().unwrap();
        assert_eq!(third.bytes, Bytes::from_static(b"third"));
        drop((first, third));
        assert_eq!(runtime.used(), 0);
    }

    #[test]
    fn failed_growth_keeps_old_snapshot_and_closing_releases_reservation() {
        let registry = LatestRegistry::default();
        let runtime = ByteBudget::new(8);
        let session = ByteBudget::new(8);
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let target = SessionTarget::Tcp(sender);
        registry
            .enqueue(
                8,
                1,
                Bytes::from_static(b"old"),
                &target,
                &runtime,
                &session,
            )
            .unwrap();
        assert_eq!(
            registry
                .enqueue(
                    8,
                    1,
                    Bytes::from_static(b"too-long-new"),
                    &target,
                    &runtime,
                    &session
                )
                .unwrap_err()
                .code(),
            rnet_core::ErrorCode::WouldBlock
        );
        assert_eq!(runtime.used(), 3);
        registry.forget_session(8);
        drop(receiver);
        assert_eq!(runtime.used(), 0);
    }

    #[test]
    fn concurrent_same_key_admission_keeps_one_bounded_pending_frame() {
        let registry = Arc::new(LatestRegistry::default());
        let runtime = ByteBudget::new(16);
        let session = ByteBudget::new(16);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let target = Arc::new(SessionTarget::Tcp(sender));
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8u8)
            .map(|byte| {
                let registry = Arc::clone(&registry);
                let runtime = Arc::clone(&runtime);
                let session = Arc::clone(&session);
                let target = Arc::clone(&target);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    registry.enqueue(
                        42,
                        1,
                        Bytes::from(vec![byte; 4]),
                        &target,
                        &runtime,
                        &session,
                    )
                })
            })
            .collect();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == LatestSendOutcome::Queued)
                .count(),
            1
        );
        assert_eq!(registry.replacements(), 7);
        assert_eq!(runtime.used(), 4);
        assert_eq!(session.used(), 4);
        let delivered = receiver.try_recv().unwrap().resolve_latest().unwrap();
        assert_eq!(delivered.bytes.len(), 4);
        assert!(delivered
            .bytes
            .iter()
            .all(|byte| *byte == delivered.bytes[0]));
        assert!(receiver.try_recv().is_err());
        drop(delivered);
        assert_eq!(runtime.used(), 0);
        assert_eq!(session.used(), 0);
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.pending_replaced, 7);
        assert_eq!(snapshot.worker_pickups, 1);
    }

    #[test]
    fn latest_admission_failures_are_split_by_stable_reason() {
        let registry = LatestRegistry::default();
        for code in [
            rnet_core::ErrorCode::WouldBlock,
            rnet_core::ErrorCode::InvalidHandle,
            rnet_core::ErrorCode::InvalidState,
            rnet_core::ErrorCode::HandshakeRequired,
            rnet_core::ErrorCode::NotSupported,
            rnet_core::ErrorCode::MessageTooLarge,
            rnet_core::ErrorCode::IoError,
        ] {
            registry.record_admission_failure(code);
        }
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.admission_would_block, 1);
        assert_eq!(snapshot.admission_invalid_handle, 1);
        assert_eq!(snapshot.admission_invalid_state, 1);
        assert_eq!(snapshot.admission_handshake_required, 1);
        assert_eq!(snapshot.admission_not_supported, 1);
        assert_eq!(snapshot.admission_message_too_large, 1);
        assert_eq!(snapshot.admission_other_failures, 1);
    }

    #[test]
    fn public_latest_send_records_rejected_admission() {
        let runtime = crate::NetworkRuntime::new(crate::RuntimeConfig::default()).unwrap();
        assert_eq!(
            runtime
                .send_payload_latest(0xdead_beef, 1, b"snapshot")
                .unwrap_err()
                .code(),
            rnet_core::ErrorCode::InvalidHandle
        );
        assert_eq!(
            runtime.latest_transport_snapshot().admission_invalid_handle,
            1
        );
        runtime.stop(std::time::Duration::ZERO).unwrap();
    }
}
