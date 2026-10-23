use crate::abi::{RnetLogger, RNET_ABI_VERSION};
use rnet_transport::RuntimeConfig;
use std::mem::size_of;

/// The complete current runtime configuration. No legacy layouts are accepted.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
/// Runtime resource/security settings; initialize with rnet_config_init, then customize.
/// Copied at creation; editing this struct afterwards does not reconfigure a runtime.
pub struct RnetConfig {
    /// Set to sizeof this exact struct; initializers fill it when provided.
    pub struct_size: u32,
    /// Set to RNET_ABI_VERSION; only the exact current layout is accepted.
    pub abi_version: u32,
    /// Positive I/O worker count; initialize defaults with rnet_config_init.
    pub worker_threads: u32,
    /// Positive transport event count limit; includes lifecycle events.
    pub event_queue_capacity: u32,
    /// Positive outbound command queue capacity in entries.
    pub write_queue_capacity: u32,
    /// Positive logical frame body limit in bytes; game envelopes consume this budget too.
    pub max_body_len: u32,
    /// UDP/KCP datagram byte ceiling including protocol/security overhead; 20..65507, default 1200.
    pub max_datagram_size: u32,
    /// Positive total queued event payload budget in bytes.
    pub max_event_bytes: u64,
    /// Positive runtime-wide send byte budget including framing reservations.
    pub max_runtime_queued_bytes: u64,
    /// Positive per-session send budget, at most max_runtime_queued_bytes.
    pub max_session_queued_bytes: u64,
    /// Positive active/pending session cap per listener/endpoint.
    pub max_sessions_per_endpoint: u32,
    /// Positive peer admission cap per IPv4 address or configured IPv6 prefix.
    pub max_sessions_per_ip: u32,
    /// Positive handshake token refill rate per second and IP/prefix.
    pub handshake_rate_per_ip: u32,
    /// Positive handshake token-bucket burst allowance per IP/prefix.
    pub handshake_burst_per_ip: u32,
    /// IPv6 admission prefix length, 0..128; zero groups all IPv6 peers, IPv4 uses full addresses.
    pub ipv6_admission_prefix_bits: u32,
    /// Positive cryptographic/application authorization deadline in milliseconds.
    pub handshake_timeout_ms: u64,
    /// Positive TCP connection-attempt timeout in milliseconds.
    pub connect_timeout_ms: u64,
    /// Positive synchronous DNS resolution wait limit in milliseconds.
    pub dns_timeout_ms: u64,
    /// Positive established UDP/KCP idle timeout in milliseconds.
    pub datagram_idle_timeout_ms: u64,
    /// Zero rejects local server plaintext mode; nonzero permits it. Clients follow server policy.
    pub allow_plaintext_business_data: u32,
    /// Server automatic key-rotation age in milliseconds; zero disables this trigger.
    pub rekey_after_ms: u64,
    /// Server automatic protected-byte rotation threshold; zero disables this trigger.
    pub rekey_after_bytes: u64,
    /// Optional logger descriptor, copied at creation; callback context remains caller-owned.
    pub logger: *const RnetLogger,
    /// Zero disables TCP_NODELAY; nonzero enables it; initializer enables it.
    pub tcp_nodelay: u32,
    /// Requested TCP kernel send buffer bytes; zero keeps OS defaults.
    pub tcp_send_buffer_bytes: u64,
    /// Requested TCP kernel receive buffer bytes; zero keeps OS defaults.
    pub tcp_recv_buffer_bytes: u64,
    /// Positive runtime endpoint cap, including listeners and outbound client endpoints.
    pub max_endpoints: u32,
    /// Positive runtime-wide pending-handshake admission cap.
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
