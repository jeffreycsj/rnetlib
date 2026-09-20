//! Additive game-facing ABI layouts. The existing transport ABI remains frozen.

use crate::abi::{RnetSlice, RNET_ABI_VERSION};
use crate::abi_config::RnetConfigV5;
use rnet_game::{NetworkQuality, QualityBasis, QualityGrade};
use std::mem::size_of;

pub const RNET_GAME_RUNTIME_STARTED: u32 = 1;
pub const RNET_GAME_ENDPOINT_OPENED: u32 = 2;
pub const RNET_GAME_ENDPOINT_ERROR: u32 = 3;
pub const RNET_GAME_AUTH_REQUEST: u32 = 4;
pub const RNET_GAME_RESUME_REQUEST: u32 = 5;
pub const RNET_GAME_PROTOCOL_REJECTED: u32 = 6;
pub const RNET_GAME_SESSION_READY: u32 = 7;
pub const RNET_GAME_SESSION_RESUMED: u32 = 8;
pub const RNET_GAME_RESUME_TICKET: u32 = 9;
pub const RNET_GAME_SESSION_CLOSED: u32 = 10;
pub const RNET_GAME_MESSAGE: u32 = 11;
pub const RNET_GAME_WRITABLE: u32 = 12;
pub const RNET_GAME_JOIN_FAILED: u32 = 13;
pub const RNET_GAME_SECURITY_CHANGED: u32 = 14;
pub const RNET_GAME_QUALITY_CHANGED: u32 = 15;
pub const RNET_GAME_PROTOCOL_VIOLATION: u32 = 16;
pub const RNET_GAME_RUNTIME_STOPPED: u32 = 17;

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

/// Borrowed event data stays valid until the corresponding buffer token is released.
/// `data` is the message, join ticket, resume ticket, or resume identity; `aux_data`
/// holds the resume request's separate join ticket. Callers must release both tokens.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameEvent {
    pub struct_size: u32,
    pub event_type: u32,
    pub endpoint: u64,
    pub session: u64,
    pub related_session: u64,
    pub status: i32,
    pub data: *const u8,
    pub data_len: usize,
    pub buffer_token: u64,
    pub aux_data: *const u8,
    pub aux_data_len: usize,
    pub aux_buffer_token: u64,
    pub client_public_key: [u8; 32],
    pub build_id: u64,
    pub capabilities: u64,
    pub encrypted: u32,
    pub security_epoch: u64,
    pub security_operation: u32,
    pub quality_grade: u32,
    pub quality_basis: u32,
    pub last_rtt_us: u64,
    pub jitter_us: u64,
    pub quality_samples: u64,
    pub has_sequence: u32,
    pub sequence: u32,
    pub has_tick: u32,
    pub tick: u32,
}

impl Default for RnetGameEvent {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            event_type: 0,
            endpoint: 0,
            session: 0,
            related_session: 0,
            status: 0,
            data: std::ptr::null(),
            data_len: 0,
            buffer_token: 0,
            aux_data: std::ptr::null(),
            aux_data_len: 0,
            aux_buffer_token: 0,
            client_public_key: [0; 32],
            build_id: 0,
            capabilities: 0,
            encrypted: 0,
            security_epoch: 0,
            security_operation: 0,
            quality_grade: 0,
            quality_basis: 0,
            last_rtt_us: 0,
            jitter_us: 0,
            quality_samples: 0,
            has_sequence: 0,
            sequence: 0,
            has_tick: 0,
            tick: 0,
        }
    }
}

/// A sample is unavailable until the first authenticated heartbeat acknowledgement.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameQuality {
    pub struct_size: u32,
    pub abi_version: u32,
    pub available: u32,
    pub grade: u32,
    pub basis: u32,
    pub has_udp_loss: u32,
    pub has_kcp_retransmissions: u32,
    pub udp_recent_loss_per_mille: u32,
    pub kcp_recent_retransmission_per_mille: u32,
    pub last_rtt_us: u64,
    pub smoothed_rtt_us: u64,
    pub jitter_us: u64,
    pub samples: u64,
    pub udp_expected: u64,
    pub udp_missing: u64,
    pub kcp_segments_sent: u64,
    pub kcp_retransmitted: u64,
}

impl Default for RnetGameQuality {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            available: 0,
            grade: 0,
            basis: 0,
            has_udp_loss: 0,
            has_kcp_retransmissions: 0,
            udp_recent_loss_per_mille: 0,
            kcp_recent_retransmission_per_mille: 0,
            last_rtt_us: 0,
            smoothed_rtt_us: 0,
            jitter_us: 0,
            samples: 0,
            udp_expected: 0,
            udp_missing: 0,
            kcp_segments_sent: 0,
            kcp_retransmitted: 0,
        }
    }
}

impl From<NetworkQuality> for RnetGameQuality {
    fn from(value: NetworkQuality) -> Self {
        let mut out = Self {
            available: 1,
            grade: quality_grade(value.grade),
            basis: quality_basis(value.basis),
            last_rtt_us: micros(value.last_rtt),
            smoothed_rtt_us: micros(value.smoothed_rtt),
            jitter_us: micros(value.jitter),
            samples: value.samples,
            ..Self::default()
        };
        if let Some(loss) = value.udp_loss {
            out.has_udp_loss = 1;
            out.udp_recent_loss_per_mille = u32::from(loss.recent_loss_per_mille);
            out.udp_expected = loss.expected;
            out.udp_missing = loss.missing;
        }
        if let Some(retransmissions) = value.kcp_retransmissions {
            out.has_kcp_retransmissions = 1;
            out.kcp_recent_retransmission_per_mille =
                u32::from(retransmissions.recent_retransmission_per_mille);
            out.kcp_segments_sent = retransmissions.segments_sent;
            out.kcp_retransmitted = retransmissions.retransmitted;
        }
        out
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RnetGameBuffer {
    pub struct_size: u32,
    pub abi_version: u32,
    pub data: *const u8,
    pub len: usize,
    pub token: u64,
}

impl Default for RnetGameBuffer {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            abi_version: RNET_ABI_VERSION,
            data: std::ptr::null(),
            len: 0,
            token: 0,
        }
    }
}

pub(crate) fn quality_grade(value: QualityGrade) -> u32 {
    match value {
        QualityGrade::Unknown => 0,
        QualityGrade::Excellent => 1,
        QualityGrade::Good => 2,
        QualityGrade::Fair => 3,
        QualityGrade::Poor => 4,
    }
}

pub(crate) fn quality_basis(value: QualityBasis) -> u32 {
    match value {
        QualityBasis::LatencyOnly => 1,
        QualityBasis::UdpSequenceGap => 2,
        QualityBasis::KcpRetransmission => 3,
    }
}

pub(crate) fn micros(value: std::time::Duration) -> u64 {
    value.as_micros().min(u128::from(u64::MAX)) as u64
}
