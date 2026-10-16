//! error contracts shared by runtime and foreign-language boundaries.

use thiserror::Error;

/// Stable local status categories shared with the C ABI; negative values denote failures.
/// Unknown numeric values fail conversion with `InvalidArgument`; these are not business errors.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    /// No local error; does not imply remote delivery or completion of an asynchronous operation.
    Ok = 0,
    /// Invalid configuration, value, pointer layout or unsupported argument combination.
    InvalidArgument = -1,
    /// Unknown, stale or already released handle in the selected owner.
    InvalidHandle = -2,
    /// Operation is not allowed in the current lifecycle or security state.
    InvalidState = -3,
    /// Bounded admission or progress is blocked; drain/poll before an appropriate retry.
    WouldBlock = -4,
    /// An operation or peer deadline expired.
    Timeout = -5,
    /// The selected transport, role or configuration does not support this operation.
    NotSupported = -6,
    /// Socket, resolver or other local I/O failure; inspect the diagnostic.
    IoError = -7,
    /// Malformed framing, unsupported wire format or invalid peer protocol behavior.
    ProtocolError = -8,
    /// A payload or declared length exceeds the applicable frame/datagram limit.
    MessageTooLarge = -9,
    /// An internal panic was caught at a library boundary; not a peer application error.
    InternalPanic = -10,
    /// The session has not completed the handshake/authorization required by the operation.
    HandshakeRequired = -11,
    /// Cryptographic establishment or the protected join exchange failed.
    HandshakeFailed = -12,
    /// The server application denied authorization.
    AuthRejected = -13,
    /// The server identity did not match the client's trusted identity.
    PeerKeyMismatch = -14,
    /// A protected datagram was repeated or fell outside the accepted replay window.
    ReplayDetected = -15,
    /// Encryption, decryption or authentication failed.
    CryptoError = -16,
    /// A configured admission or request-rate limit rejected the operation.
    RateLimited = -17,
    /// An operation or session was explicitly cancelled.
    Cancelled = -18,
}

impl TryFrom<i32> for ErrorCode {
    type Error = RnetError;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Ok),
            -1 => Ok(Self::InvalidArgument),
            -2 => Ok(Self::InvalidHandle),
            -3 => Ok(Self::InvalidState),
            -4 => Ok(Self::WouldBlock),
            -5 => Ok(Self::Timeout),
            -6 => Ok(Self::NotSupported),
            -7 => Ok(Self::IoError),
            -8 => Ok(Self::ProtocolError),
            -9 => Ok(Self::MessageTooLarge),
            -10 => Ok(Self::InternalPanic),
            -11 => Ok(Self::HandshakeRequired),
            -12 => Ok(Self::HandshakeFailed),
            -13 => Ok(Self::AuthRejected),
            -14 => Ok(Self::PeerKeyMismatch),
            -15 => Ok(Self::ReplayDetected),
            -16 => Ok(Self::CryptoError),
            -17 => Ok(Self::RateLimited),
            -18 => Ok(Self::Cancelled),
            _ => Err(RnetError::new(Self::InvalidArgument, "unknown error code")),
        }
    }
}

/// Owned diagnostic plus stable error category. Display includes both.
/// Messages are not automatically redacted: constructors must not include credentials or payloads.
#[derive(Debug, Error)]
#[error("{code:?}: {message}")]
pub struct RnetError {
    code: ErrorCode,
    message: String,
}

impl RnetError {
    /// Creates an error with caller-supplied `code` and owned diagnostic `message`.
    /// No validation/redaction is performed; use a failure code and omit secret data.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Returns the stable category independently of diagnostic wording.
    pub const fn code(&self) -> ErrorCode {
        self.code
    }
}

impl From<std::io::Error> for RnetError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorCode::IoError, error.to_string())
    }
}

/// Library result carrying a stable category and owned diagnostic on failure.
pub type Result<T> = std::result::Result<T, RnetError>;
