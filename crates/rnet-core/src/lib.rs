//! Core types shared by the transport and FFI layers.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use thiserror::Error;

pub type Handle = u64;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    Ok = 0,
    InvalidArgument = -1,
    InvalidHandle = -2,
    InvalidState = -3,
    WouldBlock = -4,
    Timeout = -5,
    NotSupported = -6,
    IoError = -7,
    ProtocolError = -8,
    MessageTooLarge = -9,
    InternalPanic = -10,
    HandshakeRequired = -11,
    HandshakeFailed = -12,
    AuthRejected = -13,
    PeerKeyMismatch = -14,
    ReplayDetected = -15,
    CryptoError = -16,
    RateLimited = -17,
    Cancelled = -18,
}

impl TryFrom<i32> for ErrorCode {
    type Error = RnetError;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Ok),
            -1 => Ok(Self::InvalidArgument),
            -2 => Ok(Self::InvalidHandle),
            -3 => Ok(Self::InvalidState),
            -4 => Ok(Self::WouldBlock),
            -5 => Ok(Self::Timeout),
            -6 => Ok(Self::NotSupported),
            -7 => Ok(Self::IoError),
            -8 => Ok(Self::ProtocolError),
            -9 => Ok(Self::MessageTooLarge),
            -10 => Ok(Self::InternalPanic),
            -11 => Ok(Self::HandshakeRequired),
            -12 => Ok(Self::HandshakeFailed),
            -13 => Ok(Self::AuthRejected),
            -14 => Ok(Self::PeerKeyMismatch),
            -15 => Ok(Self::ReplayDetected),
            -16 => Ok(Self::CryptoError),
            -17 => Ok(Self::RateLimited),
            -18 => Ok(Self::Cancelled),
            _ => Err(RnetError::new(Self::InvalidArgument, "unknown error code")),
        }
    }
}

#[derive(Debug, Error)]
#[error("{code:?}: {message}")]
pub struct RnetError {
    code: ErrorCode,
    message: String,
}

impl RnetError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }
}

impl From<std::io::Error> for RnetError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorCode::IoError, error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RnetError>;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    Tcp = 1,
    Udp = 2,
    Kcp = 3,
}

impl TryFrom<u32> for Transport {
    type Error = RnetError;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            1 => Ok(Self::Tcp),
            2 => Ok(Self::Udp),
            3 => Ok(Self::Kcp),
            _ => Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "unknown transport",
            )),
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventType {
    RuntimeStarted = 1,
    EndpointOpened = 2,
    EndpointError = 3,
    SessionOpened = 4,
    SessionClosed = 5,
    Message = 6,
    Writable = 7,
    RuntimeStopped = 8,
    AuthRequest = 9,
    JoinFailed = 10,
    /// A server-directed security mode switch or rekey completed.
    SecurityChanged = 11,
    /// An authenticated game-library control, never a business message.
    GameControl = 12,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Event {
    pub event_type: EventType,
    pub endpoint: Handle,
    pub session: Handle,
    pub msg_type: u32,
    pub stream_id: u32,
    pub request_id: u64,
    pub status: ErrorCode,
    pub data: Vec<u8>,
    /// Whether the transport verified this message record's cryptographic integrity.
    /// This describes the received record, not the session's current security mode.
    #[doc(hidden)]
    pub integrity_verified: bool,
    #[doc(hidden)]
    pub queued_at: Instant,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Event")
            .field("event_type", &self.event_type)
            .field("endpoint", &self.endpoint)
            .field("session", &self.session)
            .field("msg_type", &self.msg_type)
            .field("stream_id", &self.stream_id)
            .field("request_id", &self.request_id)
            .field("status", &self.status)
            .field("data_len", &self.data.len())
            .field("integrity_verified", &self.integrity_verified)
            .field("queued_at", &self.queued_at)
            .finish()
    }
}

impl Event {
    pub fn simple(event_type: EventType) -> Self {
        Self {
            event_type,
            endpoint: 0,
            session: 0,
            msg_type: 0,
            stream_id: 0,
            request_id: 0,
            status: ErrorCode::Ok,
            data: Vec::new(),
            integrity_verified: false,
            queued_at: Instant::now(),
        }
    }
}

#[derive(Debug)]
struct EventQueueInner {
    capacity: usize,
    byte_capacity: usize,
    queued_bytes: Mutex<usize>,
    queue: Mutex<VecDeque<Event>>,
    available: Condvar,
    space: Condvar,
}

#[derive(Clone, Debug)]
pub struct EventQueue(Arc<EventQueueInner>);

impl EventQueue {
    pub fn new(capacity: usize) -> Result<Self> {
        Self::new_with_limits(capacity, usize::MAX)
    }

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
    /// notification was evicted.
    pub fn push_priority(&self, mut event: Event) -> Result<bool> {
        let mut queue = self.0.queue.lock().expect("event queue poisoned");
        let mut queued_bytes = self
            .0
            .queued_bytes
            .lock()
            .expect("event queue byte counter poisoned");
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

    pub fn len(&self) -> usize {
        self.0.queue.lock().expect("event queue poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn queued_bytes(&self) -> usize {
        *self
            .0
            .queued_bytes
            .lock()
            .expect("event queue byte counter poisoned")
    }
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

#[derive(Debug, Default)]
pub struct HandleTable<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
}

impl<T> HandleTable<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|slot| slot.value.is_none())
    }

    pub fn insert(&mut self, value: T) -> Handle {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot {
                    generation: 1,
                    value: None,
                });
                self.slots.len() - 1
            }
        };
        let slot = &mut self.slots[index];
        slot.value = Some(value);
        encode_handle(index, slot.generation)
    }

    pub fn get(&self, handle: Handle) -> Option<&T> {
        let (index, generation) = decode_handle(handle)?;
        let slot = self.slots.get(index)?;
        (slot.generation == generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }

    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let (index, generation) = decode_handle(handle)?;
        let slot = self.slots.get_mut(index)?;
        (slot.generation == generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }

    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let (index, generation) = decode_handle(handle)?;
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(index);
        Some(value)
    }

    pub fn handles(&self) -> impl Iterator<Item = Handle> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.value
                .as_ref()
                .map(|_| encode_handle(index, slot.generation))
        })
    }
}

fn encode_handle(index: usize, generation: u32) -> Handle {
    (u64::from(generation) << 32) | (index as u64 + 1)
}

fn decode_handle(handle: Handle) -> Option<(usize, u32)> {
    let encoded_index = (handle & u64::from(u32::MAX)) as u32;
    let generation = (handle >> 32) as u32;
    if encoded_index == 0 || generation == 0 {
        return None;
    }
    Some(((encoded_index - 1) as usize, generation))
}

#[derive(Clone, Copy, Debug)]
pub struct BufferView {
    pub token: Handle,
    pub ptr: *const u8,
    pub len: usize,
}

#[derive(Debug, Default)]
pub struct BufferStore {
    buffers: Mutex<HandleTable<Box<[u8]>>>,
}

impl BufferStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores bytes and returns a stable address valid until the matching token is released.
    ///
    /// Callers must serialize release with all reads through `ptr`; releasing a token while
    /// another thread reads its pointer violates the external API contract.
    pub fn insert(&self, bytes: Vec<u8>) -> BufferView {
        let boxed = bytes.into_boxed_slice();
        let ptr = if boxed.is_empty() {
            std::ptr::null()
        } else {
            boxed.as_ptr()
        };
        let len = boxed.len();
        let token = self
            .buffers
            .lock()
            .expect("buffer store poisoned")
            .insert(boxed);
        BufferView { token, ptr, len }
    }

    pub fn release(&self, token: Handle) -> Result<()> {
        self.buffers
            .lock()
            .expect("buffer store poisoned")
            .remove(token)
            .map(drop)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid buffer token"))
    }

    pub fn copy(&self, token: Handle) -> Result<Vec<u8>> {
        self.buffers
            .lock()
            .expect("buffer store poisoned")
            .get(token)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid buffer token"))
    }

    pub fn outstanding(&self) -> usize {
        self.buffers
            .lock()
            .expect("buffer store poisoned")
            .handles()
            .count()
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Created = 0,
    Running = 1,
    Draining = 2,
    Stopped = 3,
}

pub struct RuntimeState(AtomicU8);

impl fmt::Debug for RuntimeState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RuntimeState")
            .field(&self.load())
            .finish()
    }
}

impl RuntimeState {
    pub const fn created() -> Self {
        Self(AtomicU8::new(Lifecycle::Created as u8))
    }

    pub fn load(&self) -> Lifecycle {
        match self.0.load(Ordering::Acquire) {
            0 => Lifecycle::Created,
            1 => Lifecycle::Running,
            2 => Lifecycle::Draining,
            _ => Lifecycle::Stopped,
        }
    }

    pub fn start(&self) -> Result<()> {
        self.transition(Lifecycle::Created, Lifecycle::Running)
    }

    pub fn begin_draining(&self) -> Result<()> {
        match self.load() {
            Lifecycle::Running => self.transition(Lifecycle::Running, Lifecycle::Draining),
            Lifecycle::Draining => Ok(()),
            _ => Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime cannot begin draining from this state",
            )),
        }
    }

    pub fn mark_stopped(&self) -> Result<()> {
        match self.load() {
            Lifecycle::Draining => self.transition(Lifecycle::Draining, Lifecycle::Stopped),
            Lifecycle::Stopped => Ok(()),
            _ => Err(RnetError::new(
                ErrorCode::InvalidState,
                "runtime cannot stop from this state",
            )),
        }
    }

    fn transition(&self, from: Lifecycle, to: Lifecycle) -> Result<()> {
        self.0
            .compare_exchange(from as u8, to as u8, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| {
                RnetError::new(ErrorCode::InvalidState, "invalid runtime state transition")
            })
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
