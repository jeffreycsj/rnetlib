//! Owned game-facing events; session handles are local to the runtime that emitted them.

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
    /// Borrows the credential until this value is dropped. Copies made by the caller are
    /// outside the library's zeroization guarantee and must not be logged.
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
    /// Borrows the opaque ticket for a resume call. Keep it secret; cloning or copying bytes
    /// does not extend the server-side expiry or make a one-use ticket reusable.
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
    /// Round-trip time of the most recent accepted heartbeat, including local queue delays.
    pub last_rtt: Duration,
    /// Exponentially smoothed heartbeat RTT, not a latency percentile.
    pub smoothed_rtt: Duration,
    /// Smoothed absolute change between consecutive RTT samples, not statistical variance.
    pub jitter: Duration,
    /// Accepted heartbeat sample count for this session; zero is not measured zero latency.
    pub samples: u64,
    /// Quality classification uses only signals listed by `basis`.
    pub grade: QualityGrade,
    /// Signals actually eligible for classification given the configured sample minima.
    pub basis: QualityBasis,
    /// Receiver-side data-sequence gaps. `None` for TCP/KCP, not zero loss.
    pub udp_loss: Option<UdpLossSnapshot>,
    /// Transport retransmission ratio, not raw network loss; `None` for UDP/TCP.
    pub kcp_retransmissions: Option<KcpRetransmissionSnapshot>,
}

/// Worst grade among eligible RTT, jitter and transport-specific signals in [`crate::QualityPolicy`].
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum QualityGrade {
    /// Too few RTT samples to classify; does not mean disconnected.
    Unknown,
    /// Every eligible signal is within its excellent threshold.
    Excellent,
    /// At least one signal exceeds excellent, but all remain within good thresholds.
    Good,
    /// At least one signal exceeds good, but all remain within fair thresholds.
    Fair,
    /// At least one eligible signal exceeds its fair threshold.
    Poor,
}

/// Inputs used for a quality grade; unavailable loss is never interpreted as zero loss.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualityBasis {
    /// Authenticated RTT and jitter only; also used before enough transport samples exist.
    LatencyOnly,
    /// RTT/jitter plus the receiver's recent UDP application-data sequence gaps.
    UdpSequenceGap,
    /// RTT/jitter plus recent KCP PUSH retransmissions, not raw IP-layer loss.
    KcpRetransmission,
}

/// Opaque business bytes received from an established game session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameMessage {
    /// Local listener or client endpoint that owns `session`.
    pub endpoint: Handle,
    /// Local game session from which this message was received; not a player identity.
    pub session: Handle,
    /// Sender-supplied optional sequence metadata; not the internal UDP loss sequence.
    pub sequence: Option<u32>,
    /// Simulation tick supplied by the sender, without any business-type interpretation.
    pub tick: Option<u32>,
    /// Optional request/trace correlation metadata; zero means absent.
    pub correlation_id: u64,
    /// Owned opaque application bytes, valid independently of subsequent polls.
    /// Business message types belong here. Empty payloads are permitted.
    pub payload: Bytes,
}

/// Events emitted to game code. No variant exposes `msg_type`, `stream_id`, or transport routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameEvent {
    /// Transport workers have started; no listener or game session is implied.
    RuntimeStarted,
    /// An endpoint has opened; clients must still wait for game readiness before sending.
    EndpointOpened {
        /// Local endpoint returned by listen/connect.
        endpoint: Handle,
    },
    /// Endpoint-scoped error, not necessarily terminal (for example, address failover).
    /// Query endpoint validity before treating it as closed; the logger carries the local cause.
    EndpointError {
        /// Affected local endpoint, not a session handle.
        endpoint: Handle,
        /// Local failure category; the configured logger carries diagnostic detail.
        status: ErrorCode,
    },
    /// Server-side authorization request after cryptographic and protocol checks.
    /// The application must call [`crate::GameRuntime::auth_decide`] once for this session.
    AuthRequest {
        /// Listener that accepted this request.
        endpoint: Handle,
        /// Pending local session to pass to `auth_decide`; not yet game-ready.
        session: Handle,
        /// Client static public key authenticated by the Noise handshake.
        client_public_key: [u8; 32],
        /// Client-provided login credential; the application must validate its meaning.
        join_ticket: SensitiveBytes,
        /// Authenticated build metadata; application policy may reject it.
        build_id: u64,
        /// Client-defined capability bits; not a library authorization decision.
        capabilities: u64,
    },
    /// A valid one-time claim still requires the game server to call `auth_decide`.
    ResumeRequest {
        /// Listener receiving the replacement connection.
        endpoint: Handle,
        /// New pending session to authorize, distinct from `old_session`.
        session: Handle,
        /// Original server-side handle bound to the ticket; may already be disconnected.
        old_session: Handle,
        /// Opaque identity supplied by the server when issuing the ticket; reauthorize it.
        identity: SensitiveBytes,
        /// Authenticated client key, checked against the ticket's original key.
        client_public_key: [u8; 32],
        /// Fresh application credential from this join, separate from the resume ticket.
        join_ticket: SensitiveBytes,
        /// Build metadata of the reconnecting client.
        build_id: u64,
        /// Capability metadata of the reconnecting client.
        capabilities: u64,
    },
    /// The library rejected an incompatible or malformed game join before application auth.
    ProtocolRejected {
        /// Listener that rejected the join.
        endpoint: Handle,
        /// Rejected pending session; cannot be accepted by application authorization.
        session: Handle,
        /// Rejection category; typically a protocol mismatch or malformed join.
        reason: ErrorCode,
    },
    /// Authentication and game negotiation completed; application sends may now be attempted.
    SessionReady {
        /// Local endpoint owning the ready session.
        endpoint: Handle,
        /// Ready local session handle for send and session operations.
        session: Handle,
    },
    /// The new session is ready; the old network handle is invalid on this runtime.
    SessionResumed {
        /// Local endpoint owning the replacement session.
        endpoint: Handle,
        /// Retired local handle; remove it from application routing tables.
        old_session: Handle,
        /// Ready replacement handle; business identity remains application-owned.
        new_session: Handle,
    },
    /// A short-lived credential; applications may persist it but must never log it.
    ResumeTicket {
        /// Client endpoint receiving the credential.
        endpoint: Handle,
        /// Current session for which the server issued the ticket.
        session: Handle,
        /// Opaque one-use credential for the same server runtime and client key.
        ticket: ResumeTicket,
    },
    /// A session closed; subsequent sends cannot rely on its handle remaining valid.
    SessionClosed {
        /// Endpoint that owned the session; shared server listeners may remain active.
        endpoint: Handle,
        /// Closed local session.
        session: Handle,
        /// Local close category, not an application-level disconnect reason from the peer.
        reason: ErrorCode,
    },
    /// Opaque application data from a game-ready peer.
    Message(GameMessage),
    /// Transport backpressure has eased; a retry may still fail if another sender fills it.
    /// This does not report game staging capacity or delivery of a previous message.
    Writable {
        /// Endpoint owning the writable session.
        endpoint: Handle,
        /// Local session on which a send may be retried.
        session: Handle,
    },
    /// Connection/handshake failed before transport opening; no ready game session is implied.
    JoinFailed {
        /// Local client endpoint associated with the failed join.
        endpoint: Handle,
        /// Associated session, or zero if failure preceded session allocation.
        session: Handle,
        /// Local failure category; detailed diagnostics are available through logging.
        reason: ErrorCode,
    },
    /// Local transport applied an authenticated security transition.
    /// Events on the two peers may be polled at different times.
    SecurityChanged {
        /// Endpoint owning the affected session.
        endpoint: Handle,
        /// Local session whose business-data protection changed or whose keys rotated.
        session: Handle,
        /// Whether the resulting business-data mode is encrypted; controls stay protected.
        encrypted: bool,
        /// Session-local security generation, not a timestamp or application version.
        epoch: u64,
        /// Whether the transition changed mode or rotated keys.
        operation: SecurityOperation,
    },
    /// Emitted only after two consecutive non-unknown samples support a changed grade/basis.
    QualityChanged {
        /// Endpoint owning the sampled session.
        endpoint: Handle,
        /// Local session whose stabilized quality classification changed.
        session: Handle,
        /// Measurements and classification at the time of this event, not a live view.
        quality: NetworkQuality,
    },
    /// Game processing failed closed, including invalid protected input or exhausted local
    /// pre-ready buffering. Invalid unverified plaintext envelopes are counted and dropped instead.
    ProtocolViolation {
        /// Endpoint that received the failed input; the listener is not closed by this event.
        endpoint: Handle,
        /// Affected session; a close is requested. Logs carry the underlying failure detail.
        session: Handle,
    },
    /// Runtime shutdown completed; no further sends should be attempted.
    RuntimeStopped,
}
