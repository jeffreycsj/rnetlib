//! Bounded, key-coalescing pending queue for optional game-state snapshots.

use bytes::Bytes;
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use std::collections::{HashMap, VecDeque};

const WORST_ENVELOPE_OVERHEAD: usize = 16;

/// Limits are in addition to the transport's own bounded send queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RealtimeQueueConfig {
    pub max_queued_bytes: usize,
    pub max_session_queued_bytes: usize,
    pub max_keys_per_session: usize,
    pub flush_batch: usize,
}

impl Default for RealtimeQueueConfig {
    fn default() -> Self {
        Self {
            max_queued_bytes: 16 * 1024 * 1024,
            max_session_queued_bytes: 256 * 1024,
            max_keys_per_session: 64,
            flush_batch: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueueEffect {
    Queued,
    Replaced,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RealtimeQueueSnapshot {
    pub queued_messages: usize,
    pub queued_bytes: usize,
    pub admission_rejected: u64,
    pub replaced: u64,
    pub closed_dropped: u64,
    pub backpressure_dropped: u64,
    pub send_failed: u64,
    pub forwarded: u64,
}

pub(crate) struct PendingRealtime {
    pub(crate) session: Handle,
    pub(crate) key: u64,
    pub(crate) payload: Bytes,
    pub(crate) tick: Option<u32>,
}

impl PendingRealtime {
    fn reserved_bytes(&self) -> usize {
        self.payload.len() + WORST_ENVELOPE_OVERHEAD
    }
}

pub(crate) struct LatestQueue {
    max_total_bytes: usize,
    max_session_bytes: usize,
    max_keys_per_session: usize,
    max_pending_keys: usize,
    order: VecDeque<(Handle, u64)>,
    pending: HashMap<(Handle, u64), PendingRealtime>,
    session_usage: HashMap<Handle, (usize, usize)>,
    counters: RealtimeQueueSnapshot,
}

impl LatestQueue {
    pub(crate) fn from_config(config: RealtimeQueueConfig) -> Result<Self> {
        if config.flush_batch == 0 {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "realtime flush batch must be positive",
            ));
        }
        Self::new(
            config.max_queued_bytes,
            config.max_session_queued_bytes,
            config.max_keys_per_session,
        )
    }
    pub(crate) fn new(
        max_total_bytes: usize,
        max_session_bytes: usize,
        max_keys_per_session: usize,
    ) -> Result<Self> {
        if max_total_bytes == 0
            || max_session_bytes == 0
            || max_session_bytes > max_total_bytes
            || max_keys_per_session == 0
        {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid realtime queue limits",
            ));
        }
        Ok(Self {
            max_total_bytes,
            max_session_bytes,
            max_keys_per_session,
            // Empty payloads still own map, order, and session entries. This cap prevents a
            // large byte budget from permitting millions of tiny-key allocations.
            max_pending_keys: (max_total_bytes / WORST_ENVELOPE_OVERHEAD).clamp(1, 65_536),
            order: VecDeque::new(),
            pending: HashMap::new(),
            session_usage: HashMap::new(),
            counters: RealtimeQueueSnapshot::default(),
        })
    }

    pub(crate) fn enqueue(
        &mut self,
        session: Handle,
        key: u64,
        payload: Bytes,
        tick: Option<u32>,
    ) -> Result<QueueEffect> {
        let candidate = PendingRealtime {
            session,
            key,
            payload,
            tick,
        };
        let new_bytes = candidate.reserved_bytes();
        let old_bytes = self
            .pending
            .get(&(session, key))
            .map_or(0, PendingRealtime::reserved_bytes);
        let (session_keys, session_bytes) = self
            .session_usage
            .get(&session)
            .copied()
            .unwrap_or_default();
        let new_total = self
            .counters
            .queued_bytes
            .saturating_sub(old_bytes)
            .checked_add(new_bytes);
        let new_session = session_bytes
            .saturating_sub(old_bytes)
            .checked_add(new_bytes);
        if new_total.is_none_or(|bytes| bytes > self.max_total_bytes)
            || new_session.is_none_or(|bytes| bytes > self.max_session_bytes)
            || (old_bytes == 0 && session_keys >= self.max_keys_per_session)
            || (old_bytes == 0 && self.pending.len() >= self.max_pending_keys)
        {
            self.counters.admission_rejected = self.counters.admission_rejected.saturating_add(1);
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "realtime queue budget is full",
            ));
        }
        let effect = if self.pending.insert((session, key), candidate).is_some() {
            self.counters.replaced = self.counters.replaced.saturating_add(1);
            QueueEffect::Replaced
        } else {
            self.order.push_back((session, key));
            self.counters.queued_messages += 1;
            QueueEffect::Queued
        };
        self.counters.queued_bytes = new_total.expect("validated total bytes");
        self.session_usage.insert(
            session,
            (
                session_keys + usize::from(effect == QueueEffect::Queued),
                new_session.expect("validated session bytes"),
            ),
        );
        Ok(effect)
    }

    pub(crate) fn pop_next(&mut self) -> Option<PendingRealtime> {
        while let Some(key) = self.order.pop_front() {
            if let Some(message) = self.pending.remove(&key) {
                self.release(&message);
                return Some(message);
            }
        }
        None
    }

    /// A blocked send rotates behind other sessions; a newer value always wins the key.
    pub(crate) fn requeue(&mut self, message: PendingRealtime) {
        if self.pending.contains_key(&(message.session, message.key)) {
            self.counters.replaced = self.counters.replaced.saturating_add(1);
            return;
        }
        if self
            .enqueue(message.session, message.key, message.payload, message.tick)
            .is_err()
        {
            self.counters.backpressure_dropped =
                self.counters.backpressure_dropped.saturating_add(1);
        }
    }

    pub(crate) fn forget_session(&mut self, session: Handle) -> usize {
        let keys: Vec<_> = self
            .pending
            .keys()
            .filter(|(owner, _)| *owner == session)
            .copied()
            .collect();
        for key in &keys {
            if let Some(message) = self.pending.remove(key) {
                self.release(&message);
            }
        }
        self.order.retain(|(owner, _)| *owner != session);
        self.counters.closed_dropped = self
            .counters
            .closed_dropped
            .saturating_add(keys.len() as u64);
        keys.len()
    }

    pub(crate) fn clear(&mut self) {
        self.counters.closed_dropped = self
            .counters
            .closed_dropped
            .saturating_add(self.pending.len() as u64);
        self.counters.queued_messages = 0;
        self.counters.queued_bytes = 0;
        self.pending.clear();
        self.order.clear();
        self.session_usage.clear();
    }

    pub(crate) fn record_forwarded(&mut self) {
        self.counters.forwarded = self.counters.forwarded.saturating_add(1);
    }

    pub(crate) fn record_send_failed(&mut self) {
        self.counters.send_failed = self.counters.send_failed.saturating_add(1);
    }

    pub(crate) fn snapshot(&self) -> RealtimeQueueSnapshot {
        self.counters
    }

    fn release(&mut self, message: &PendingRealtime) {
        let bytes = message.reserved_bytes();
        self.counters.queued_messages -= 1;
        self.counters.queued_bytes -= bytes;
        if let Some((keys, used)) = self.session_usage.get_mut(&message.session) {
            *keys -= 1;
            *used -= bytes;
            if *keys == 0 {
                self.session_usage.remove(&message.session);
            }
        }
    }
}
