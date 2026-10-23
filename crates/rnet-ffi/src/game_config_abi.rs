//! C layouts for game runtime, listener, and client configuration.

use crate::abi::{RnetLogger, RnetSlice, RNET_ABI_VERSION};
use crate::abi_config::RnetConfig;
use std::mem::size_of;

/// Complete game configuration. Zero queue budgets select production defaults.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Game runtime options. Initialize first; zero fallback semantics are field-specific.
pub struct RnetGameConfig {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// Probe interval in milliseconds; both heartbeat fields zero select defaults (5 s / 15 s).
    pub heartbeat_interval_ms: u32,
    /// Acknowledgement timeout; if either heartbeat field is nonzero, both must be positive and timeout >= interval.
    pub heartbeat_timeout_ms: u32,
    /// 0/1 server plaintext permission; overrides the same field in network_config.
    pub allow_plaintext_business_data: u32,
    /// Reserved; must be zero.
    pub reserved: u32,
    /// NULL uses transport defaults; otherwise copied. Its logger must be NULL: use this config's logger.
    pub network_config: *const RnetConfig,
    /// Snapshot staging bytes across runtime; zero selects 16 MiB.
    pub realtime_max_queued_bytes: u64,
    /// Per-session snapshot staging bytes; zero selects 256 KiB; cannot exceed total.
    pub realtime_max_session_queued_bytes: u64,
    /// Distinct pending snapshot keys per session; zero selects 64.
    pub realtime_max_keys_per_session: u64,
    /// Snapshot forwarding attempts per poll; zero selects 256.
    pub realtime_flush_batch: u64,
    /// Ordinary/priority staging bytes across runtime; zero selects 16 MiB.
    pub scheduled_max_queued_bytes: u64,
    /// Per-session ordinary/priority staging bytes; zero selects 256 KiB; cannot exceed total.
    pub scheduled_max_session_queued_bytes: u64,
    /// Ordinary/priority staging message cap; zero selects 65,536.
    pub scheduled_max_queued_messages: u64,
    /// Ordinary/priority forwarding attempts per poll; zero selects 256.
    pub scheduled_flush_batch: u64,
    /// Optional structured logger; callback data must live until runtime destruction.
    /// Records omit payloads/credentials/tickets and report this public game runtime handle.
    /// The callback may query metrics but must not stop or destroy a runtime.
    /// Optional async logger descriptor, copied at creation; userdata lives until destroy returns.
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

/// Inclusive application versions; equal bounds select an exact version.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Listener policy and inclusive business-version range; copied by listen.
pub struct RnetGameServerConfig {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// RNET_TRANSPORT_* fixed for the listener and accepted sessions.
    pub transport: u32,
    /// 0 plaintext or 1 encrypted business data; 0 requires runtime plaintext permission.
    pub initial_encryption: u32,
    /// UTF-8 numeric IPv4/IPv6 only, without brackets, URL scheme or embedded port.
    pub bind_host: RnetSlice,
    /// Local listening port; zero requests an OS-assigned port.
    pub bind_port: u16,
    /// Reserved; must be zero.
    pub reserved: u16,
    /// Exactly 32 server identity bytes, copied by listen.
    pub local_private_key: RnetSlice,
    /// Nonzero application protocol identity, equal on both peers; not an SDK/transport version.
    pub protocol_id: u64,
    /// Nonzero lowest supported application version, inclusive.
    pub min_version: u32,
    /// Highest supported version, inclusive and >= min_version; equal bounds require an exact version.
    pub max_version: u32,
}

/// Explicit wire-v4 hostname join, including metadata authenticated in the join payload.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Outbound game join metadata. Connect returns an endpoint, not a ready session.
pub struct RnetGameClientConfig {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// RNET_TRANSPORT_* matching the remote listener; fixed after connect.
    pub transport: u32,
    /// Reserved; must be zero.
    pub reserved: u32,
    /// UTF-8 DNS hostname or numeric IP, not a URL; port is supplied separately.
    pub remote_host: RnetSlice,
    /// Remote listening port; nonzero.
    pub remote_port: u16,
    /// Reserved; must be zero.
    pub reserved2: u16,
    /// Opaque application credential, copied on connect; can be empty if server authorization allows.
    pub join_ticket: RnetSlice,
    /// Nonzero application protocol identity, matching the listener.
    pub protocol_id: u64,
    /// Nonzero minimum application version, inclusive.
    pub min_version: u32,
    /// Maximum application version, inclusive and >= min_version.
    pub max_version: u32,
    /// Reserved; must be zero.
    pub reserved3: u32,
    /// Authenticated client build metadata; zero unspecified, not interpreted by the library.
    pub build_id: u64,
    /// Authenticated application-defined capability bits; zero none, not library feature grants.
    pub capabilities: u64,
}
