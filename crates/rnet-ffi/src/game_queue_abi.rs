//! C snapshots for bounded game-stage send queues.

use crate::abi::RNET_ABI_VERSION;
use rnet_game::{RealtimeQueueSnapshot, ScheduledQueueSnapshot};
use std::mem::size_of;

/// Runtime-wide staging gauges and cumulative counters. Forwarded means admitted to the
/// transport send queue, not delivered to the peer; no per-player labels or payloads appear.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRealtimeQueue {
    pub struct_size: u32,
    pub abi_version: u32,
    pub queued_messages: u64,
    pub queued_bytes: u64,
    pub admission_rejected: u64,
    pub replaced: u64,
    pub closed_dropped: u64,
    pub backpressure_dropped: u64,
    pub send_failed: u64,
    pub forwarded: u64,
}

impl Default for RnetGameRealtimeQueue {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            queued_messages: 0,
            queued_bytes: 0,
            admission_rejected: 0,
            replaced: 0,
            closed_dropped: 0,
            backpressure_dropped: 0,
            send_failed: 0,
            forwarded: 0,
        }
    }
}

impl From<RealtimeQueueSnapshot> for RnetGameRealtimeQueue {
    fn from(value: RealtimeQueueSnapshot) -> Self {
        Self {
            queued_messages: u64::try_from(value.queued_messages).unwrap_or(u64::MAX),
            queued_bytes: u64::try_from(value.queued_bytes).unwrap_or(u64::MAX),
            admission_rejected: value.admission_rejected,
            replaced: value.replaced,
            closed_dropped: value.closed_dropped,
            backpressure_dropped: value.backpressure_dropped,
            send_failed: value.send_failed,
            forwarded: value.forwarded,
            ..Self::default()
        }
    }
}

/// Runtime-wide priority/expiry staging gauges and cumulative counters.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameScheduledQueue {
    pub struct_size: u32,
    pub abi_version: u32,
    pub queued_messages: u64,
    pub queued_bytes: u64,
    pub admission_rejected: u64,
    pub expired_dropped: u64,
    pub closed_dropped: u64,
    pub send_failed: u64,
    pub forwarded: u64,
    pub backpressure_requeued: u64,
    pub admitted_by_priority: [u64; 4],
    pub forwarded_by_priority: [u64; 4],
    pub queue_delay_samples: u64,
    pub queue_delay_p90_us: u64,
    pub queue_delay_p95_us: u64,
    pub queue_delay_p99_us: u64,
    pub queue_delay_max_us: u64,
    pub tick_queue_delay_samples: u64,
    pub tick_queue_delay_p90_us: u64,
    pub tick_queue_delay_p95_us: u64,
    pub tick_queue_delay_p99_us: u64,
    pub tick_queue_delay_max_us: u64,
}

impl Default for RnetGameScheduledQueue {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            queued_messages: 0,
            queued_bytes: 0,
            admission_rejected: 0,
            expired_dropped: 0,
            closed_dropped: 0,
            send_failed: 0,
            forwarded: 0,
            backpressure_requeued: 0,
            admitted_by_priority: [0; 4],
            forwarded_by_priority: [0; 4],
            queue_delay_samples: 0,
            queue_delay_p90_us: 0,
            queue_delay_p95_us: 0,
            queue_delay_p99_us: 0,
            queue_delay_max_us: 0,
            tick_queue_delay_samples: 0,
            tick_queue_delay_p90_us: 0,
            tick_queue_delay_p95_us: 0,
            tick_queue_delay_p99_us: 0,
            tick_queue_delay_max_us: 0,
        }
    }
}

impl From<ScheduledQueueSnapshot> for RnetGameScheduledQueue {
    fn from(value: ScheduledQueueSnapshot) -> Self {
        Self {
            queued_messages: u64::try_from(value.queued_messages).unwrap_or(u64::MAX),
            queued_bytes: u64::try_from(value.queued_bytes).unwrap_or(u64::MAX),
            admission_rejected: value.admission_rejected,
            expired_dropped: value.expired_dropped,
            closed_dropped: value.closed_dropped,
            send_failed: value.send_failed,
            forwarded: value.forwarded,
            backpressure_requeued: value.backpressure_requeued,
            admitted_by_priority: value.admitted_by_priority,
            forwarded_by_priority: value.forwarded_by_priority,
            queue_delay_samples: value.queue_delay.sample_count,
            queue_delay_p90_us: value.queue_delay.p90_us,
            queue_delay_p95_us: value.queue_delay.p95_us,
            queue_delay_p99_us: value.queue_delay.p99_us,
            queue_delay_max_us: value.queue_delay.max_us,
            tick_queue_delay_samples: value.tick_queue_delay.sample_count,
            tick_queue_delay_p90_us: value.tick_queue_delay.p90_us,
            tick_queue_delay_p95_us: value.tick_queue_delay.p95_us,
            tick_queue_delay_p99_us: value.tick_queue_delay.p99_us,
            tick_queue_delay_max_us: value.tick_queue_delay.max_us,
            ..Self::default()
        }
    }
}
