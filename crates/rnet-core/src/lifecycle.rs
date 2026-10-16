//! lifecycle contracts shared by runtime and foreign-language boundaries.

use crate::{ErrorCode, Result, RnetError};
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

/// Monotonic runtime lifecycle; state changes alone do not perform I/O cleanup.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    /// Constructed but not started.
    Created = 0,
    /// Accepting normal work.
    Running = 1,
    /// Shutdown has begun; stop admission and settle already accepted work.
    Draining = 2,
    /// Terminal stopped state; cannot restart this state machine.
    Stopped = 3,
}

/// Atomic lifecycle flag with acquire/release publication, not a lock for runtime resources.
/// The owner must coordinate admission, queues and workers around these transitions.
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
    /// Creates a state machine in `Created` without starting workers.
    pub const fn created() -> Self {
        Self(AtomicU8::new(Lifecycle::Created as u8))
    }

    /// Reads the lifecycle with acquire ordering.
    pub fn load(&self) -> Lifecycle {
        match self.0.load(Ordering::Acquire) {
            0 => Lifecycle::Created,
            1 => Lifecycle::Running,
            2 => Lifecycle::Draining,
            _ => Lifecycle::Stopped,
        }
    }

    /// Atomically changes Created to Running; repeated or competing transitions return `InvalidState`.
    pub fn start(&self) -> Result<()> {
        self.transition(Lifecycle::Created, Lifecycle::Running)
    }

    /// Transitions Running to Draining; observing Draining is a no-op.
    /// Other states or a lost compare-exchange race return `InvalidState`; does not drain queues.
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

    /// Transitions Draining to Stopped; observing Stopped is a no-op.
    /// Other states or a lost compare-exchange race return `InvalidState`; does not join workers.
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
