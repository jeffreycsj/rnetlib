//! event contracts shared by runtime and foreign-language boundaries.

use crate::{ErrorCode, Handle, Result, RnetError};
use std::time::Instant;

/// Transport selected once for an endpoint; sends address a session rather than choosing a transport.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    /// Reliable ordered stream with library framing; delivery still depends on session survival.
    Tcp = 1,
    /// Unreliable datagrams; no retransmission, ordering or delivery guarantee.
    Udp = 2,
    /// Reliable ordered KCP messages carried over UDP.
    Kcp = 3,
}

impl TryFrom<u32> for Transport {
    type Error = RnetError;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            1 => Ok(Self::Tcp),
            2 => Ok(Self::Udp),
            3 => Ok(Self::Kcp),
            _ => Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "unknown transport",
            )),
        }
    }
}

/// Transport event discriminant; consult it before interpreting the reused event fields.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventType {
    /// Workers started; endpoint/session are zero, not a peer-readiness notification.
    RuntimeStarted = 1,
    /// Endpoint socket opened; no ready peer is implied.
    EndpointOpened = 2,
    /// Endpoint-related failure, possibly recoverable; inspect status and endpoint validity.
    EndpointError = 3,
    /// Transport handshake and authorization completed; game negotiation may still be pending.
    SessionOpened = 4,
    /// Session closed; status is the local reason and data may contain optional diagnostics.
    SessionClosed = 5,
    /// Opaque business bytes with optional correlation metadata.
    Message = 6,
    /// Advisory send-capacity recovery; a subsequent send may still encounter backpressure.
    Writable = 7,
    /// Runtime stopped; already exported buffers still belong to their holders.
    RuntimeStopped = 8,
    /// Server authorization required; data is the authenticated 32-byte client key followed by join bytes.
    AuthRequest = 9,
    /// Outbound connection/handshake failed; inspect local status and optional diagnostics.
    JoinFailed = 10,
    /// A server-directed security mode switch or rekey completed.
    SecurityChanged = 11,
    /// An authenticated game-library control, never a business message.
    GameControl = 12,
}

/// Opaque message events expose correlation, never business type or stream routing.
///
/// ```compile_fail
/// let event = rnet_core::Event::simple(rnet_core::EventType::Message);
/// let _ = event.msg_type;
/// ```
/// ```compile_fail
/// let event = rnet_core::Event::simple(rnet_core::EventType::Message);
/// let _ = event.stream_id;
/// ```
/// ```compile_fail
/// let event = rnet_core::Event::simple(rnet_core::EventType::Message);
/// let _ = event.request_id;
/// ```
#[derive(Clone, Eq, PartialEq)]
pub struct Event {
    /// Discriminant determining which fields/payload shape are meaningful.
    pub event_type: EventType,
    /// Local owning endpoint; zero for runtime-wide notifications.
    pub endpoint: Handle,
    /// Local peer handle; zero when no session applies or allocation never occurred.
    pub session: Handle,
    /// Optional caller correlation carried by message events; zero means unspecified.
    pub correlation_id: u64,
    /// Local failure/closure category; `Ok` on non-error events, not a business reply code.
    pub status: ErrorCode,
    /// Owned business bytes or kind-specific control/authorization/diagnostic payload.
    /// Debug omits these bytes, but clones create independent copies; erase sensitive copies explicitly.
    pub data: Vec<u8>,
    /// Whether the transport verified this message record's cryptographic integrity.
    /// This describes the received record, not the session's current security mode.
    #[doc(hidden)]
    pub integrity_verified: bool,
    /// Monotonic enqueue timestamp for local queue-latency measurement; overwritten on admission.
    #[doc(hidden)]
    pub queued_at: Instant,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Event")
            .field("event_type", &self.event_type)
            .field("endpoint", &self.endpoint)
            .field("session", &self.session)
            .field("correlation_id", &self.correlation_id)
            .field("status", &self.status)
            .field("data_len", &self.data.len())
            .field("integrity_verified", &self.integrity_verified)
            .field("queued_at", &self.queued_at)
            .finish()
    }
}

impl Event {
    /// Creates an empty event of `event_type` with zero handles/correlation and status `Ok`.
    /// Integrity starts false; construction itself proves neither receipt nor authentication.
    pub fn simple(event_type: EventType) -> Self {
        Self {
            event_type,
            endpoint: 0,
            session: 0,
            correlation_id: 0,
            status: ErrorCode::Ok,
            data: Vec::new(),
            integrity_verified: false,
            queued_at: Instant::now(),
        }
    }
}
