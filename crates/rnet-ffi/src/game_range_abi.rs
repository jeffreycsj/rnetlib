//! Wire-v4 game range ABI layouts kept separate from the general game facade ABI.

use crate::abi::RNET_ABI_VERSION;
use rnet_game::RangeBufferSnapshot;
use std::mem::size_of;

/// Runtime-wide wire-v4 early-data usage without endpoint, session, or player labels.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Early-data buffering during business-version negotiation, scoped to this runtime.
pub struct RnetGameRangeBuffer {
    /// Output layout size in bytes, written by the library.
    pub struct_size: u32,
    /// Output ABI identifier, written by the library.
    pub abi_version: u32,
    /// Current early messages retained, including ready-but-not-yet-polled messages.
    pub buffered_messages: u64,
    /// Current early payload bytes; excludes framing/allocator overhead.
    pub buffered_bytes: u64,
    /// Highest retained early-message count observed.
    pub peak_buffered_messages: u64,
    /// Highest retained early payload bytes observed.
    pub peak_buffered_bytes: u64,
    /// Runtime-wide cap inherited from configured event queue capacity.
    pub max_buffered_messages: u64,
    /// Runtime-wide cap inherited from configured event-byte budget.
    pub max_buffered_bytes: u64,
    /// Cumulative per-session early-data budget rejections.
    pub session_admission_rejected: u64,
    /// Cumulative runtime-wide early-data budget rejections.
    pub runtime_admission_rejected: u64,
}

impl Default for RnetGameRangeBuffer {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            buffered_messages: 0,
            buffered_bytes: 0,
            peak_buffered_messages: 0,
            peak_buffered_bytes: 0,
            max_buffered_messages: 0,
            max_buffered_bytes: 0,
            session_admission_rejected: 0,
            runtime_admission_rejected: 0,
        }
    }
}

impl From<RangeBufferSnapshot> for RnetGameRangeBuffer {
    fn from(value: RangeBufferSnapshot) -> Self {
        Self {
            buffered_messages: u64::try_from(value.buffered_messages).unwrap_or(u64::MAX),
            buffered_bytes: u64::try_from(value.buffered_bytes).unwrap_or(u64::MAX),
            peak_buffered_messages: u64::try_from(value.peak_buffered_messages).unwrap_or(u64::MAX),
            peak_buffered_bytes: u64::try_from(value.peak_buffered_bytes).unwrap_or(u64::MAX),
            max_buffered_messages: u64::try_from(value.max_buffered_messages).unwrap_or(u64::MAX),
            max_buffered_bytes: u64::try_from(value.max_buffered_bytes).unwrap_or(u64::MAX),
            session_admission_rejected: value.session_admission_rejected,
            runtime_admission_rejected: value.runtime_admission_rejected,
            ..Self::default()
        }
    }
}
