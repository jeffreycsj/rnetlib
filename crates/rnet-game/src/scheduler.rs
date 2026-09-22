//! Bounded weighted scheduler for optional game-send priorities and local expiry.

use bytes::Bytes;
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use rnet_observe::{LatencyHistogram, LatencySnapshot};
use std::array;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

const ENVELOPE_RESERVATION: usize = 16;
const WEIGHTED_ORDER: [usize; 15] = [3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 1, 1, 0];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GamePriority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

impl GamePriority {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::High => 2,
            Self::Critical => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledQueueConfig {
    pub max_queued_bytes: usize,
    pub max_session_queued_bytes: usize,
    pub max_queued_messages: usize,
    pub flush_batch: usize,
}

impl Default for ScheduledQueueConfig {
    fn default() -> Self {
        Self {
            max_queued_bytes: 16 * 1024 * 1024,
            max_session_queued_bytes: 256 * 1024,
            max_queued_messages: 65_536,
            flush_batch: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScheduledQueueSnapshot {
    pub queued_messages: usize,
    pub queued_bytes: usize,
    pub admission_rejected: u64,
    pub expired_dropped: u64,
    pub closed_dropped: u64,
    pub send_failed: u64,
    pub forwarded: u64,
    pub backpressure_requeued: u64,
    pub admitted_by_priority: [u64; 4],
    pub forwarded_by_priority: [u64; 4],
    /// Local admission-to-transport delay, not remote delivery latency.
    pub queue_delay: LatencySnapshot,
    /// Queue delay for messages that carry a simulation tick.
    pub tick_queue_delay: LatencySnapshot,
}

pub(crate) struct ScheduledMessage {
    pub(crate) session: Handle,
    pub(crate) payload: Bytes,
    pub(crate) sequence: Option<u32>,
    pub(crate) tick: Option<u32>,
    pub(crate) correlation_id: u64,
    pub(crate) priority: GamePriority,
    pub(crate) enqueued_at: Instant,
    pub(crate) expires_at: Option<Instant>,
}

impl ScheduledMessage {
    fn reserved_bytes(&self) -> usize {
        self.payload.len().saturating_add(ENVELOPE_RESERVATION)
    }
}

pub(crate) struct ScheduledQueue {
    queues: [FairPriorityQueue; 4],
    session_bytes: HashMap<Handle, usize>,
    max_total_bytes: usize,
    max_session_bytes: usize,
    max_messages: usize,
    cursor: usize,
    counters: ScheduledQueueSnapshot,
    queue_delay: LatencyHistogram,
    tick_queue_delay: LatencyHistogram,
}

#[derive(Default)]
struct FairPriorityQueue {
    sessions: VecDeque<Handle>,
    messages: HashMap<Handle, VecDeque<ScheduledMessage>>,
}

impl FairPriorityQueue {
    fn push(&mut self, message: ScheduledMessage) {
        let session = message.session;
        let queue = self.messages.entry(session).or_default();
        if queue.is_empty() {
            self.sessions.push_back(session);
        }
        queue.push_back(message);
    }

    fn pop(&mut self) -> Option<ScheduledMessage> {
        let session = self.sessions.pop_front()?;
        let queue = self
            .messages
            .get_mut(&session)
            .expect("scheduled session order references a queue");
        let message = queue
            .pop_front()
            .expect("scheduled session queue is nonempty");
        if queue.is_empty() {
            self.messages.remove(&session);
        } else {
            self.sessions.push_back(session);
        }
        Some(message)
    }

    fn remove_session(&mut self, session: Handle) -> usize {
        self.sessions.retain(|queued| *queued != session);
        self.messages
            .remove(&session)
            .map_or(0, |queue| queue.len())
    }

    fn clear(&mut self) {
        self.sessions.clear();
        self.messages.clear();
    }
}

impl ScheduledQueue {
    pub(crate) fn from_config(config: ScheduledQueueConfig) -> Result<Self> {
        if config.max_queued_bytes == 0
            || config.max_session_queued_bytes == 0
            || config.max_session_queued_bytes > config.max_queued_bytes
            || config.max_queued_messages == 0
            || config.flush_batch == 0
        {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid scheduled queue limits",
            ));
        }
        Ok(Self {
            queues: array::from_fn(|_| FairPriorityQueue::default()),
            session_bytes: HashMap::new(),
            max_total_bytes: config.max_queued_bytes,
            max_session_bytes: config.max_session_queued_bytes,
            max_messages: config.max_queued_messages,
            cursor: 0,
            counters: ScheduledQueueSnapshot::default(),
            queue_delay: LatencyHistogram::default(),
            tick_queue_delay: LatencyHistogram::default(),
        })
    }

    pub(crate) fn enqueue(&mut self, message: ScheduledMessage) -> Result<()> {
        let bytes = message.reserved_bytes();
        let session_bytes = self
            .session_bytes
            .get(&message.session)
            .copied()
            .unwrap_or_default();
        let fits = self.counters.queued_messages < self.max_messages
            && self
                .counters
                .queued_bytes
                .checked_add(bytes)
                .is_some_and(|total| total <= self.max_total_bytes)
            && session_bytes
                .checked_add(bytes)
                .is_some_and(|total| total <= self.max_session_bytes);
        if !fits {
            self.counters.admission_rejected = self.counters.admission_rejected.saturating_add(1);
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "scheduled game queue budget is full",
            ));
        }
        self.counters.queued_messages += 1;
        self.counters.queued_bytes += bytes;
        let priority = message.priority.index();
        self.counters.admitted_by_priority[priority] =
            self.counters.admitted_by_priority[priority].saturating_add(1);
        self.session_bytes
            .insert(message.session, session_bytes + bytes);
        self.queues[message.priority.index()].push(message);
        Ok(())
    }

    pub(crate) fn pop_next(&mut self, now: Instant) -> Option<ScheduledMessage> {
        loop {
            let mut selected = None;
            for _ in 0..WEIGHTED_ORDER.len() {
                let priority = WEIGHTED_ORDER[self.cursor];
                self.cursor = (self.cursor + 1) % WEIGHTED_ORDER.len();
                if let Some(message) = self.queues[priority].pop() {
                    selected = Some(message);
                    break;
                }
            }
            let message = selected?;
            if message.expires_at.is_some_and(|deadline| now >= deadline) {
                self.release(&message);
                self.counters.expired_dropped = self.counters.expired_dropped.saturating_add(1);
                continue;
            }
            return Some(message);
        }
    }

    pub(crate) fn requeue(&mut self, message: ScheduledMessage) {
        self.counters.backpressure_requeued = self.counters.backpressure_requeued.saturating_add(1);
        let priority = message.priority.index();
        self.queues[priority].push(message);
    }

    pub(crate) fn forget_session(&mut self, session: Handle) -> usize {
        let mut dropped = 0usize;
        for queue in &mut self.queues {
            dropped += queue.remove_session(session);
        }
        if let Some(bytes) = self.session_bytes.remove(&session) {
            self.counters.queued_bytes = self.counters.queued_bytes.saturating_sub(bytes);
        }
        self.counters.queued_messages = self.counters.queued_messages.saturating_sub(dropped);
        self.counters.closed_dropped = self.counters.closed_dropped.saturating_add(dropped as u64);
        dropped
    }

    pub(crate) fn record_forwarded(&mut self, message: &ScheduledMessage) {
        let delay = message.enqueued_at.elapsed();
        self.queue_delay.record(delay);
        if message.tick.is_some() {
            self.tick_queue_delay.record(delay);
        }
        self.release(message);
        self.counters.forwarded = self.counters.forwarded.saturating_add(1);
        let priority = message.priority.index();
        self.counters.forwarded_by_priority[priority] =
            self.counters.forwarded_by_priority[priority].saturating_add(1);
    }

    pub(crate) fn record_send_failed(&mut self, message: &ScheduledMessage) {
        self.release(message);
        self.counters.send_failed = self.counters.send_failed.saturating_add(1);
    }

    pub(crate) fn clear(&mut self) {
        let dropped = self.counters.queued_messages;
        self.queues.iter_mut().for_each(FairPriorityQueue::clear);
        self.session_bytes.clear();
        self.counters.queued_messages = 0;
        self.counters.queued_bytes = 0;
        self.counters.closed_dropped = self.counters.closed_dropped.saturating_add(dropped as u64);
    }

    pub(crate) fn snapshot(&self) -> ScheduledQueueSnapshot {
        ScheduledQueueSnapshot {
            queue_delay: self.queue_delay.snapshot(),
            tick_queue_delay: self.tick_queue_delay.snapshot(),
            ..self.counters
        }
    }

    fn release(&mut self, message: &ScheduledMessage) {
        let bytes = message.reserved_bytes();
        self.counters.queued_messages = self.counters.queued_messages.saturating_sub(1);
        self.counters.queued_bytes = self.counters.queued_bytes.saturating_sub(bytes);
        if let Some(session_bytes) = self.session_bytes.get_mut(&message.session) {
            *session_bytes = session_bytes.saturating_sub(bytes);
            if *session_bytes == 0 {
                self.session_bytes.remove(&message.session);
            }
        }
    }
}
