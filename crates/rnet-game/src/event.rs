//! Game-facing events that hide compatibility framing and transport details.

use crate::quality::UdpLossSnapshot;
use bytes::Bytes;
use rnet_core::{ErrorCode, Handle};
use rnet_transport::{KcpRetransmissionSnapshot, SecurityOperation};
use std::time::Duration;
use zeroize::Zeroize;

/// Sensitive application bytes exposed to authorization callbacks. Debug output is redacted,
/// and the owned copy is erased when the event is dropped.
#[derive(Clone, Eq, PartialEq)]
pub struct SensitiveBytes(Vec<u8>);

impl SensitiveBytes {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for SensitiveBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for SensitiveBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SensitiveBytes(<redacted>)")
    }
}

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::ops::Deref for SensitiveBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for SensitiveBytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.0.as_slice() == other.as_slice()
    }
}

/// Bearer credential delivered only over authenticated game control. Debug output is redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct ResumeTicket(Vec<u8>);

impl ResumeTicket {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for ResumeTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResumeTicket(<redacted>)")
    }
}

impl Drop for ResumeTicket {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::ops::Deref for ResumeTicket {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

/// Last authenticated heartbeat sample for one established game session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkQuality {
    pub last_rtt: Duration,
    pub smoothed_rtt: Duration,
    pub jitter: Duration,
    pub samples: u64,
    /// Quality classification uses only signals listed by `basis`.
    pub grade: QualityGrade,
    pub basis: QualityBasis,
    /// Receiver-side data-sequence gaps. `None` for TCP/KCP, not zero loss.
    pub udp_loss: Option<UdpLossSnapshot>,
    /// Transport retransmission ratio, not raw network loss; `None` for UDP/TCP.
    pub kcp_retransmissions: Option<KcpRetransmissionSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum QualityGrade {
    Unknown,
    Excellent,
    Good,
    Fair,
    Poor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualityBasis {
    LatencyOnly,
    UdpSequenceGap,
    KcpRetransmission,
}

/// Opaque business bytes received from an established game session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameMessage {
    pub endpoint: Handle,
    pub session: Handle,
    /// Network-owned sequence metadata, when requested by the advanced send API.
    pub sequence: Option<u32>,
    /// Simulation tick supplied by the sender, without any business-type interpretation.
    pub tick: Option<u32>,
    pub payload: Bytes,
}

/// Events emitted to game code. No variant exposes `msg_type`, `stream_id`, or transport routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameEvent {
    RuntimeStarted,
    EndpointOpened {
        endpoint: Handle,
    },
    EndpointError {
        endpoint: Handle,
        status: ErrorCode,
    },
    AuthRequest {
        endpoint: Handle,
        session: Handle,
        client_public_key: [u8; 32],
        join_ticket: SensitiveBytes,
        /// Authenticated build metadata; application policy may reject it.
        build_id: u64,
        capabilities: u64,
    },
    /// A valid one-time claim still requires the game server to call `auth_decide`.
    ResumeRequest {
        endpoint: Handle,
        session: Handle,
        old_session: Handle,
        identity: SensitiveBytes,
        client_public_key: [u8; 32],
        join_ticket: SensitiveBytes,
        build_id: u64,
        capabilities: u64,
    },
    /// The library rejected an incompatible or malformed game join before application auth.
    ProtocolRejected {
        endpoint: Handle,
        session: Handle,
        reason: ErrorCode,
    },
    SessionReady {
        endpoint: Handle,
        session: Handle,
    },
    /// The new session is ready; the old network handle is invalid on this runtime.
    SessionResumed {
        endpoint: Handle,
        old_session: Handle,
        new_session: Handle,
    },
    /// A short-lived credential; applications may persist it but must never log it.
    ResumeTicket {
        endpoint: Handle,
        session: Handle,
        ticket: ResumeTicket,
    },
    SessionClosed {
        endpoint: Handle,
        session: Handle,
        reason: ErrorCode,
    },
    Message(GameMessage),
    Writable {
        endpoint: Handle,
        session: Handle,
    },
    JoinFailed {
        endpoint: Handle,
        session: Handle,
        reason: ErrorCode,
    },
    SecurityChanged {
        endpoint: Handle,
        session: Handle,
        encrypted: bool,
        epoch: u64,
        operation: SecurityOperation,
    },
    /// Emitted only after two consecutive non-unknown samples support a changed grade/basis.
    QualityChanged {
        endpoint: Handle,
        session: Handle,
        quality: NetworkQuality,
    },
    /// The peer sent framing that cannot be produced by this game API.
    ProtocolViolation {
        endpoint: Handle,
        session: Handle,
    },
    RuntimeStopped,
}
