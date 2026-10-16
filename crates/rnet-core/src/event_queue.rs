//! event queue contracts shared by runtime and foreign-language boundaries.

use crate::{ErrorCode, Event, EventType, Result, RnetError};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct EventQueueInner {
    capacity: usize,
    byte_capacity: usize,
    // Whenever both locks are needed, acquire queue before queued_bytes. A capacity
    // waiter releases the byte lock before sleeping so polling can return budget.
    queued_bytes: Mutex<usize>,
    queue: Mutex<VecDeque<Event>>,
    available: Condvar,
    space: Condvar,
}

/// Thread-safe bounded FIFO shared by clones, with separate count and payload-byte limits.
/// Budget covers queued `data.len()`, not allocator overhead or events already returned by poll.
/// Priority publication may evict data/control notifications; lifecycle delivery is not guaranteed.
#[derive(Clone, Debug)]
pub struct EventQueue(Arc<EventQueueInner>);

impl EventQueue {
    /// Creates a positive event-count limit with an effectively unlimited byte budget.
    /// Zero returns `InvalidArgument`; use `new_with_limits` for untrusted payloads.
    pub fn new(capacity: usize) -> Result<Self> {
        Self::new_with_limits(capacity, usize::MAX)
    }

    /// Creates count and payload-byte limits; either zero returns `InvalidArgument`.
    /// Capacity is a limit, not a request to preallocate all event slots.
    pub fn new_with_limits(capacity: usize, byte_capacity: usize) -> Result<Self> {
        if capacity == 0 || byte_capacity == 0 {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "event queue count and byte capacities must be positive",
            ));
        }
        Ok(Self(Arc::new(EventQueueInner {
            capacity,
            byte_capacity,
            queued_bytes: Mutex::new(0),
            queue: Mutex::new(VecDeque::new()),
            available: Condvar::new(),
            space: Condvar::new(),
        })))
    }

    /// Attempts immediate admission, taking ownership of `event` even on failure.
    /// Count/byte exhaustion or byte-addition overflow returns `WouldBlock`; no old event is evicted.
    pub fn try_push(&self, mut event: Event) -> Result<()> {
        let mut queue = self.0.queue.lock().expect("event queue poisoned");
        let mut queued_bytes = self
            .0
            .queued_bytes
            .lock()
            .expect("event queue byte counter poisoned");
        if queue.len() == self.0.capacity
            || queued_bytes
                .checked_add(event.data.len())
                .is_none_or(|total| total > self.0.byte_capacity)
        {
            return Err(RnetError::new(ErrorCode::WouldBlock, "event queue is full"));
        }
        *queued_bytes += event.data.len();
        event.queued_at = Instant::now();
        queue.push_back(event);
        self.0.available.notify_one();
        Ok(())
    }

    /// Publishes a state-transition event without exceeding the configured capacity.
    ///
    /// When the queue is full, an older data/writability notification is evicted first. If the
    /// queue contains only state transitions, the new transition is rejected so an already
    /// published lifecycle history is never rewritten. The return value reports whether a data
    /// notification was evicted. Optional local diagnostics on close/join/endpoint errors are
    /// discarded before they would consume unavailable bytes; their status/identity survive.
    /// `event` is consumed on error too. `WouldBlock` can occur after some data events
    /// have been evicted; rejection is not a transactional rollback of those evictions.
    pub fn push_priority(&self, mut event: Event) -> Result<bool> {
        let mut queue = self.0.queue.lock().expect("event queue poisoned");
        let mut queued_bytes = self
            .0
            .queued_bytes
            .lock()
            .expect("event queue byte counter poisoned");
        // Details must not lose a transition or evict extra messages solely for logging.
        // Authentication and other lifecycle payloads are semantic and must never be stripped.
        if matches!(
            event.event_type,
            EventType::SessionClosed | EventType::JoinFailed | EventType::EndpointError
        ) && queued_bytes
            .checked_add(event.data.len())
            .is_none_or(|total| total > self.0.byte_capacity)
        {
            event.data = Vec::new();
        }
        if event.data.len() > self.0.byte_capacity {
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "lifecycle event exceeds the event byte budget",
            ));
        }
        let mut evicted = false;
        while queue.len() == self.0.capacity
            || queued_bytes
                .checked_add(event.data.len())
                .is_none_or(|total| total > self.0.byte_capacity)
        {
            let position = queue.iter().position(|queued| {
                matches!(
                    queued.event_type,
                    EventType::Message | EventType::Writable | EventType::GameControl
                )
            });
            if let Some(position) = position {
                if let Some(removed) = queue.remove(position) {
                    *queued_bytes = queued_bytes.saturating_sub(removed.data.len());
                }
            } else {
                return Err(RnetError::new(
                    ErrorCode::WouldBlock,
                    "event queue contains only lifecycle notifications",
                ));
            }
            evicted = true;
        }
        *queued_bytes += event.data.len();
        event.queued_at = Instant::now();
        queue.push_back(event);
        self.0.available.notify_one();
        Ok(evicted)
    }

    /// Waits for count/byte capacity and takes ownership of `event`, including on error.
    /// An unrepresentable deadline returns `InvalidArgument`; an exhausted wait returns
    /// `WouldBlock`, not `Timeout`. Zero only attempts immediate admission; lock acquisition
    /// can exceed the wait budget. No existing events are evicted.
    pub fn push_timeout(&self, mut event: Event, timeout: Duration) -> Result<()> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "event queue timeout cannot form a valid deadline",
            )
        })?;
        let mut queue = self.0.queue.lock().expect("event queue poisoned");
        let mut queued_bytes = self
            .0
            .queued_bytes
            .lock()
            .expect("event queue byte counter poisoned");
        while queue.len() == self.0.capacity
            || queued_bytes
                .checked_add(event.data.len())
                .is_none_or(|total| total > self.0.byte_capacity)
        {
            drop(queued_bytes);
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RnetError::new(
                    ErrorCode::WouldBlock,
                    "event queue remained full",
                ));
            }
            let (next, wait) = self
                .0
                .space
                .wait_timeout(queue, remaining)
                .expect("event queue poisoned");
            queue = next;
            queued_bytes = self
                .0
                .queued_bytes
                .lock()
                .expect("event queue byte counter poisoned");
            if wait.timed_out()
                && (queue.len() == self.0.capacity
                    || queued_bytes
                        .checked_add(event.data.len())
                        .is_none_or(|total| total > self.0.byte_capacity))
            {
                return Err(RnetError::new(
                    ErrorCode::WouldBlock,
                    "event queue remained full",
                ));
            }
        }
        *queued_bytes += event.data.len();
        event.queued_at = Instant::now();
        queue.push_back(event);
        self.0.available.notify_one();
        Ok(())
    }

    /// Moves up to `capacity` FIFO entries out and releases their queue byte reservations.
    /// Zero capacity returns immediately. If empty, waits once for at most `timeout`;
    /// spurious wakeups or another consumer may produce an empty result. This is not a
    /// hard total-call deadline or game-maintenance driver; use the game runtime's poll.
    pub fn poll(&self, capacity: usize, timeout: Duration) -> Vec<Event> {
        if capacity == 0 {
            return Vec::new();
        }
        let mut queue = self.0.queue.lock().expect("event queue poisoned");
        if queue.is_empty() && !timeout.is_zero() {
            let (next, _) = self
                .0
                .available
                .wait_timeout(queue, timeout)
                .expect("event queue poisoned");
            queue = next;
        }

        let take = capacity.min(queue.len());
        let events: Vec<Event> = queue.drain(..take).collect();
        if take != 0 {
            let released = events.iter().map(|event| event.data.len()).sum::<usize>();
            let mut queued_bytes = self
                .0
                .queued_bytes
                .lock()
                .expect("event queue byte counter poisoned");
            *queued_bytes = queued_bytes.saturating_sub(released);
            self.0.space.notify_all();
        }
        events
    }

    /// Returns the current queued count; concurrent producers/consumers may immediately change it.
    pub fn len(&self) -> usize {
        self.0.queue.lock().expect("event queue poisoned").len()
    }

    /// Observes whether the queue is empty; does not reserve the next producer/consumer operation.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns currently charged payload bytes, excluding polled events and allocation overhead.
    /// This observation is not atomic with a separate `len` call.
    pub fn queued_bytes(&self) -> usize {
        *self
            .0
            .queued_bytes
            .lock()
            .expect("event queue byte counter poisoned")
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::EventQueue;

    #[test]
    fn event_queue_capacity_does_not_preallocate_event_slots() {
        let queue = EventQueue::new_with_limits(64, 1024).unwrap();
        let allocated = queue.0.queue.lock().unwrap().capacity();

        assert_eq!(allocated, 0);
    }
}
