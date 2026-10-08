//! C layouts for game runtime, listener, and client configuration.

use crate::abi::{RnetLogger, RnetSlice, RNET_ABI_VERSION};
use crate::abi_config::RnetConfig;
use std::mem::size_of;

/// Complete game configuration. Zero queue budgets select production defaults.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    pub allow_plaintext_business_data: u32,
    pub reserved: u32,
    pub network_config: *const RnetConfig,
    pub realtime_max_queued_bytes: u64,
    pub realtime_max_session_queued_bytes: u64,
    pub realtime_max_keys_per_session: u64,
    pub realtime_flush_batch: u64,
    pub scheduled_max_queued_bytes: u64,
    pub scheduled_max_session_queued_bytes: u64,
    pub scheduled_max_queued_messages: u64,
    pub scheduled_flush_batch: u64,
    /// Optional structured logger; callback data must live until runtime destruction.
    pub logger: *const RnetLogger,
}

impl Default for RnetGameConfig {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            heartbeat_interval_ms: 0,
            heartbeat_timeout_ms: 0,
            allow_plaintext_business_data: 0,
            reserved: 0,
            network_config: std::ptr::null(),
            realtime_max_queued_bytes: 0,
            realtime_max_session_queued_bytes: 0,
            realtime_max_keys_per_session: 0,
            realtime_flush_batch: 0,
            scheduled_max_queued_bytes: 0,
            scheduled_max_session_queued_bytes: 0,
            scheduled_max_queued_messages: 0,
            scheduled_flush_batch: 0,
            logger: std::ptr::null(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameServerConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub initial_encryption: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved: u16,
    pub local_private_key: RnetSlice,
    pub protocol_id: u64,
    pub protocol_version: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameClientConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved: u32,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved2: u16,
    pub join_ticket: RnetSlice,
    pub protocol_id: u64,
    pub protocol_version: u32,
    pub reserved3: u32,
    pub build_id: u64,
    pub capabilities: u64,
}

/// Explicit wire-v4 listener. Existing exact-version game config retains wire v3.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRangeServerConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub initial_encryption: u32,
    pub bind_host: RnetSlice,
    pub bind_port: u16,
    pub reserved: u16,
    pub local_private_key: RnetSlice,
    pub protocol_id: u64,
    pub min_version: u32,
    pub max_version: u32,
}

/// Explicit wire-v4 hostname join, including metadata authenticated in the join payload.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameRangeClientConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub transport: u32,
    pub reserved: u32,
    pub remote_host: RnetSlice,
    pub remote_port: u16,
    pub reserved2: u16,
    pub join_ticket: RnetSlice,
    pub protocol_id: u64,
    pub min_version: u32,
    pub max_version: u32,
    pub reserved3: u32,
    pub build_id: u64,
    pub capabilities: u64,
}
