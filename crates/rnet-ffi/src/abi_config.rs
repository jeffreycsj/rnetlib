use crate::abi::{RnetLogger, RNET_ABI_VERSION};
use rnet_transport::RuntimeConfig;
use std::mem::size_of;

/// The complete current runtime configuration. No legacy layouts are accepted.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub worker_threads: u32,
    pub event_queue_capacity: u32,
    pub write_queue_capacity: u32,
    pub max_body_len: u32,
    pub max_datagram_size: u32,
    pub max_event_bytes: u64,
    pub max_runtime_queued_bytes: u64,
    pub max_session_queued_bytes: u64,
    pub max_sessions_per_endpoint: u32,
    pub max_sessions_per_ip: u32,
    pub handshake_rate_per_ip: u32,
    pub handshake_burst_per_ip: u32,
    pub ipv6_admission_prefix_bits: u32,
    pub handshake_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub dns_timeout_ms: u64,
    pub datagram_idle_timeout_ms: u64,
    pub allow_plaintext_business_data: u32,
    pub allow_legacy_unauthenticated_endpoints: u32,
    pub rekey_after_ms: u64,
    pub rekey_after_bytes: u64,
    pub logger: *const RnetLogger,
    pub tcp_nodelay: u32,
    pub tcp_send_buffer_bytes: u64,
    pub tcp_recv_buffer_bytes: u64,
    pub max_endpoints: u32,
    pub max_pending_handshakes: u32,
}

impl Default for RnetConfig {
    fn default() -> Self {
        let defaults = RuntimeConfig::production();
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            worker_threads: defaults.worker_threads.min(u32::MAX as usize) as u32,
            event_queue_capacity: defaults.event_queue_capacity as u32,
            write_queue_capacity: defaults.write_queue_capacity as u32,
            max_body_len: defaults.max_body_len as u32,
            max_datagram_size: defaults.max_datagram_size as u32,
            max_event_bytes: defaults.max_event_bytes as u64,
            max_runtime_queued_bytes: defaults.max_runtime_queued_bytes as u64,
            max_session_queued_bytes: defaults.max_session_queued_bytes as u64,
            max_sessions_per_endpoint: defaults.max_sessions_per_endpoint as u32,
            max_sessions_per_ip: defaults.max_sessions_per_ip as u32,
            handshake_rate_per_ip: defaults.handshake_rate_per_ip,
            handshake_burst_per_ip: defaults.handshake_burst_per_ip,
            ipv6_admission_prefix_bits: u32::from(defaults.ipv6_admission_prefix_bits),
            handshake_timeout_ms: defaults.handshake_timeout.as_millis() as u64,
            connect_timeout_ms: defaults.connect_timeout.as_millis() as u64,
            dns_timeout_ms: defaults.dns_timeout.as_millis() as u64,
            datagram_idle_timeout_ms: defaults.datagram_idle_timeout.as_millis() as u64,
            allow_plaintext_business_data: 0,
            allow_legacy_unauthenticated_endpoints: 0,
            rekey_after_ms: defaults
                .security_policy
                .rekey_after
                .map_or(0, |duration| duration.as_millis() as u64),
            rekey_after_bytes: defaults.security_policy.rekey_after_bytes.unwrap_or(0),
            logger: std::ptr::null(),
            tcp_nodelay: u32::from(defaults.tcp_nodelay),
            tcp_send_buffer_bytes: defaults.tcp_send_buffer_bytes.unwrap_or(0) as u64,
            tcp_recv_buffer_bytes: defaults.tcp_recv_buffer_bytes.unwrap_or(0) as u64,
            max_endpoints: defaults.max_endpoints as u32,
            max_pending_handshakes: defaults.max_pending_handshakes as u32,
        }
    }
}
