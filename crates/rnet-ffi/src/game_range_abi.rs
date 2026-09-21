//! Wire-v4 game range ABI layouts kept separate from the general game facade ABI.

use crate::abi::RNET_ABI_VERSION;
use rnet_game::RangeBufferSnapshot;
use std::mem::size_of;

/// Runtime-wide wire-v4 early-data usage without endpoint, session, or player labels.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRangeBuffer {
    pub struct_size: u32,
    pub abi_version: u32,
    pub buffered_messages: u64,
    pub buffered_bytes: u64,
    pub peak_buffered_messages: u64,
    pub peak_buffered_bytes: u64,
    pub max_buffered_messages: u64,
    pub max_buffered_bytes: u64,
    pub session_admission_rejected: u64,
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
