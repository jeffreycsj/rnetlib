use crate::abi::{RnetConfigV3, RnetLogger, RnetLoggerV2};
use rnet_transport::RuntimeConfig;
use std::mem::size_of;

/// Socket-tunable production configuration. V3 remains frozen for ABI compatibility.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetConfigV4 {
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
    pub logger_v2: *const RnetLoggerV2,
    pub tcp_nodelay: u32,
    pub tcp_send_buffer_bytes: u64,
    pub tcp_recv_buffer_bytes: u64,
}

impl RnetConfigV4 {
    pub(crate) fn v3_fields(self) -> RnetConfigV3 {
        RnetConfigV3 {
            struct_size: size_of::<RnetConfigV3>() as u32,
            abi_version: self.abi_version,
            worker_threads: self.worker_threads,
            event_queue_capacity: self.event_queue_capacity,
            write_queue_capacity: self.write_queue_capacity,
            max_body_len: self.max_body_len,
            max_datagram_size: self.max_datagram_size,
            max_event_bytes: self.max_event_bytes,
            max_runtime_queued_bytes: self.max_runtime_queued_bytes,
            max_session_queued_bytes: self.max_session_queued_bytes,
            max_sessions_per_endpoint: self.max_sessions_per_endpoint,
            max_sessions_per_ip: self.max_sessions_per_ip,
            handshake_rate_per_ip: self.handshake_rate_per_ip,
            handshake_burst_per_ip: self.handshake_burst_per_ip,
            ipv6_admission_prefix_bits: self.ipv6_admission_prefix_bits,
            handshake_timeout_ms: self.handshake_timeout_ms,
            connect_timeout_ms: self.connect_timeout_ms,
            dns_timeout_ms: self.dns_timeout_ms,
            datagram_idle_timeout_ms: self.datagram_idle_timeout_ms,
            allow_plaintext_business_data: self.allow_plaintext_business_data,
            allow_legacy_unauthenticated_endpoints: self.allow_legacy_unauthenticated_endpoints,
            rekey_after_ms: self.rekey_after_ms,
            rekey_after_bytes: self.rekey_after_bytes,
            logger: self.logger,
            logger_v2: self.logger_v2,
        }
    }
}

impl Default for RnetConfigV4 {
    fn default() -> Self {
        let v3 = RnetConfigV3::default();
        let runtime = RuntimeConfig::production();
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: v3.abi_version,
            worker_threads: v3.worker_threads,
            event_queue_capacity: v3.event_queue_capacity,
            write_queue_capacity: v3.write_queue_capacity,
            max_body_len: v3.max_body_len,
            max_datagram_size: v3.max_datagram_size,
            max_event_bytes: v3.max_event_bytes,
            max_runtime_queued_bytes: v3.max_runtime_queued_bytes,
            max_session_queued_bytes: v3.max_session_queued_bytes,
            max_sessions_per_endpoint: v3.max_sessions_per_endpoint,
            max_sessions_per_ip: v3.max_sessions_per_ip,
            handshake_rate_per_ip: v3.handshake_rate_per_ip,
            handshake_burst_per_ip: v3.handshake_burst_per_ip,
            ipv6_admission_prefix_bits: v3.ipv6_admission_prefix_bits,
            handshake_timeout_ms: v3.handshake_timeout_ms,
            connect_timeout_ms: v3.connect_timeout_ms,
            dns_timeout_ms: v3.dns_timeout_ms,
            datagram_idle_timeout_ms: v3.datagram_idle_timeout_ms,
            allow_plaintext_business_data: v3.allow_plaintext_business_data,
            allow_legacy_unauthenticated_endpoints: v3.allow_legacy_unauthenticated_endpoints,
            rekey_after_ms: v3.rekey_after_ms,
            rekey_after_bytes: v3.rekey_after_bytes,
            logger: v3.logger,
            logger_v2: v3.logger_v2,
            tcp_nodelay: u32::from(runtime.tcp_nodelay),
            tcp_send_buffer_bytes: runtime.tcp_send_buffer_bytes.unwrap_or(0) as u64,
            tcp_recv_buffer_bytes: runtime.tcp_recv_buffer_bytes.unwrap_or(0) as u64,
        }
    }
}

/// Runtime-wide resource-limit extension. V3 and V4 remain frozen for ABI compatibility.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetConfigV5 {
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
    pub logger_v2: *const RnetLoggerV2,
    pub tcp_nodelay: u32,
    pub tcp_send_buffer_bytes: u64,
    pub tcp_recv_buffer_bytes: u64,
    pub max_endpoints: u32,
    pub max_pending_handshakes: u32,
}

impl RnetConfigV5 {
    pub(crate) fn v4_fields(self) -> RnetConfigV4 {
        RnetConfigV4 {
            struct_size: size_of::<RnetConfigV4>() as u32,
            abi_version: self.abi_version,
            worker_threads: self.worker_threads,
            event_queue_capacity: self.event_queue_capacity,
            write_queue_capacity: self.write_queue_capacity,
            max_body_len: self.max_body_len,
            max_datagram_size: self.max_datagram_size,
            max_event_bytes: self.max_event_bytes,
            max_runtime_queued_bytes: self.max_runtime_queued_bytes,
            max_session_queued_bytes: self.max_session_queued_bytes,
            max_sessions_per_endpoint: self.max_sessions_per_endpoint,
            max_sessions_per_ip: self.max_sessions_per_ip,
            handshake_rate_per_ip: self.handshake_rate_per_ip,
            handshake_burst_per_ip: self.handshake_burst_per_ip,
            ipv6_admission_prefix_bits: self.ipv6_admission_prefix_bits,
            handshake_timeout_ms: self.handshake_timeout_ms,
            connect_timeout_ms: self.connect_timeout_ms,
            dns_timeout_ms: self.dns_timeout_ms,
            datagram_idle_timeout_ms: self.datagram_idle_timeout_ms,
            allow_plaintext_business_data: self.allow_plaintext_business_data,
            allow_legacy_unauthenticated_endpoints: self.allow_legacy_unauthenticated_endpoints,
            rekey_after_ms: self.rekey_after_ms,
            rekey_after_bytes: self.rekey_after_bytes,
            logger: self.logger,
            logger_v2: self.logger_v2,
            tcp_nodelay: self.tcp_nodelay,
            tcp_send_buffer_bytes: self.tcp_send_buffer_bytes,
            tcp_recv_buffer_bytes: self.tcp_recv_buffer_bytes,
        }
    }
}

impl Default for RnetConfigV5 {
    fn default() -> Self {
        let v4 = RnetConfigV4::default();
        let runtime = RuntimeConfig::production();
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: v4.abi_version,
            worker_threads: v4.worker_threads,
            event_queue_capacity: v4.event_queue_capacity,
            write_queue_capacity: v4.write_queue_capacity,
            max_body_len: v4.max_body_len,
            max_datagram_size: v4.max_datagram_size,
            max_event_bytes: v4.max_event_bytes,
            max_runtime_queued_bytes: v4.max_runtime_queued_bytes,
            max_session_queued_bytes: v4.max_session_queued_bytes,
            max_sessions_per_endpoint: v4.max_sessions_per_endpoint,
            max_sessions_per_ip: v4.max_sessions_per_ip,
            handshake_rate_per_ip: v4.handshake_rate_per_ip,
            handshake_burst_per_ip: v4.handshake_burst_per_ip,
            ipv6_admission_prefix_bits: v4.ipv6_admission_prefix_bits,
            handshake_timeout_ms: v4.handshake_timeout_ms,
            connect_timeout_ms: v4.connect_timeout_ms,
            dns_timeout_ms: v4.dns_timeout_ms,
            datagram_idle_timeout_ms: v4.datagram_idle_timeout_ms,
            allow_plaintext_business_data: v4.allow_plaintext_business_data,
            allow_legacy_unauthenticated_endpoints: v4.allow_legacy_unauthenticated_endpoints,
            rekey_after_ms: v4.rekey_after_ms,
            rekey_after_bytes: v4.rekey_after_bytes,
            logger: v4.logger,
            logger_v2: v4.logger_v2,
            tcp_nodelay: v4.tcp_nodelay,
            tcp_send_buffer_bytes: v4.tcp_send_buffer_bytes,
            tcp_recv_buffer_bytes: v4.tcp_recv_buffer_bytes,
            max_endpoints: runtime.max_endpoints as u32,
            max_pending_handshakes: runtime.max_pending_handshakes as u32,
        }
    }
}
