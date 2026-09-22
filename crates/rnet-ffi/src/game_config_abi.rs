//! C layouts for game runtime, listener, and client configuration.

use crate::abi::{RnetSlice, RNET_ABI_VERSION};
use crate::abi_config::RnetConfigV5;
use std::mem::size_of;

/// Zero values on the optional timing fields select the Rust production defaults.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    pub allow_plaintext_business_data: u32,
    pub reserved: u32,
    /// Optional transport limits; borrowed only during runtime creation.
    pub network_config: *const RnetConfigV5,
}

/// Additive runtime configuration with explicit game-stage queue budgets.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameConfigV2 {
    pub struct_size: u32,
    pub abi_version: u32,
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    pub allow_plaintext_business_data: u32,
    pub reserved: u32,
    pub network_config: *const RnetConfigV5,
    pub realtime_max_queued_bytes: u64,
    pub realtime_max_session_queued_bytes: u64,
    pub realtime_max_keys_per_session: u64,
    pub realtime_flush_batch: u64,
    pub scheduled_max_queued_bytes: u64,
    pub scheduled_max_session_queued_bytes: u64,
    pub scheduled_max_queued_messages: u64,
    pub scheduled_flush_batch: u64,
}

impl Default for RnetGameConfigV2 {
    fn default() -> Self {
        let base = RnetGameConfig::default();
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: base.abi_version,
            heartbeat_interval_ms: base.heartbeat_interval_ms,
            heartbeat_timeout_ms: base.heartbeat_timeout_ms,
            allow_plaintext_business_data: base.allow_plaintext_business_data,
            reserved: 0,
            network_config: base.network_config,
            realtime_max_queued_bytes: 0,
            realtime_max_session_queued_bytes: 0,
            realtime_max_keys_per_session: 0,
            realtime_flush_batch: 0,
            scheduled_max_queued_bytes: 0,
            scheduled_max_session_queued_bytes: 0,
            scheduled_max_queued_messages: 0,
            scheduled_flush_batch: 0,
        }
    }
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
