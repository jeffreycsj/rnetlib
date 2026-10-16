//! Ownership, event queues and lifecycle primitives shared by runtime and FFI layers.
//!
//! Handles and buffer tokens are local to their owner, not globally unique capabilities.
//! Queue budgets account for queued payload bytes, not total process memory or exported buffers.
//! See the higher-level game runtime for peer readiness, heartbeats and recovery.

#![deny(missing_docs)]

mod buffer;
mod error;
mod event;
mod event_queue;
mod handle;
mod lifecycle;

pub use buffer::{BufferStore, BufferView};
pub use error::{ErrorCode, Result, RnetError};
pub use event::{Event, EventType, Transport};
pub use event_queue::EventQueue;
pub use handle::HandleTable;
pub use lifecycle::{Lifecycle, RuntimeState};

/// Owner-local opaque identifier; zero is invalid. Retain the originating runtime/store
/// and handle kind: numeric equality alone neither identifies an owner nor authorizes access.
pub type Handle = u64;
