//! Versioned records used to negotiate and change a session's security mode.
//!
//! These records are transport-independent. TCP adds its own length prefix, while UDP and KCP
//! carry exactly one encoded record per datagram or reliable message.
//! Serialization is not authentication: outer records remain untrusted until the handshake/
//! cipher and session state validate them. Only verified plaintext may enter protected/control
//! decoding. Limits below count payload bytes, not their additional record headers.

use rnet_core::{ErrorCode, Result, RnetError};

const MAGIC: u32 = 0x524e_5332;
const VERSION: u16 = 1;
const HEADER_LEN: usize = 20;
const CONTROL_LEN: usize = 10;
const PROTECTED_HEADER_LEN: usize = 5;

/// Determines whether application data is sent as plaintext or as a protected Noise record.
///
/// Protocol control messages remain protected after the initial Noise handshake even while
/// application data is in plaintext mode. This makes a later server-directed transition
/// authentic, although plaintext application data itself has no confidentiality or integrity.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityMode {
    /// Application frames are visible and unauthenticated on the wire.
    Plaintext = 1,
    /// Application frames are encrypted and authenticated with the active Noise keys.
    Encrypted = 2,
}

impl TryFrom<u8> for SecurityMode {
    type Error = RnetError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Plaintext),
            2 => Ok(Self::Encrypted),
            _ => protocol_error("unknown security mode"),
        }
    }
}

/// Identifies the outer record carried by a transport.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordKind {
    /// Starts a session using the current authenticated transport record layout.
    ClientHello = 1,
    /// Announces the server identity and initial application-data mode.
    ServerHello = 2,
    /// Carries one Noise handshake message.
    Handshake = 3,
    /// Carries an encrypted data or control message.
    Protected = 4,
    /// Carries a plaintext application frame for the declared epoch.
    PlainData = 5,
    /// Challenges an unverified datagram peer before allocating Noise state.
    CookieChallenge = 6,
}

impl TryFrom<u8> for RecordKind {
    type Error = RnetError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::ClientHello),
            2 => Ok(Self::ServerHello),
            3 => Ok(Self::Handshake),
            4 => Ok(Self::Protected),
            5 => Ok(Self::PlainData),
            6 => Ok(Self::CookieChallenge),
            _ => protocol_error("unknown security record kind"),
        }
    }
}

/// One complete current-format security record; decoding alone does not establish trust.
/// Debug includes payload bytes, so do not log records containing credentials or business data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
    /// Record interpretation. Handshake records use epoch zero; established records do not.
    pub kind: RecordKind,
    /// Monotonically increasing security generation for established sessions.
    pub epoch: u64,
    /// Kind-specific bytes. The decoder bounds this allocation using the caller's limit.
    pub payload: Vec<u8>,
}

/// Identifies plaintext content after a protected record has been authenticated and decrypted.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedKind {
    /// Carries the server's authentication decision and authenticated initial security mode.
    AuthDecision = 1,
    /// Carries one complete logical business frame.
    Data = 2,
    /// Carries an encoded [`Control`] transition barrier.
    Control = 3,
    /// Confirms receipt of a datagram authentication decision.
    AuthAck = 4,
    /// Carries a game-library control independently of the business-data security mode.
    GameControl = 5,
}

impl TryFrom<u8> for ProtectedKind {
    type Error = RnetError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::AuthDecision),
            2 => Ok(Self::Data),
            3 => Ok(Self::Control),
            4 => Ok(Self::AuthAck),
            5 => Ok(Self::GameControl),
            _ => protocol_error("unknown protected message kind"),
        }
    }
}

/// Content intended for a [`RecordKind::Protected`] record, before encryption or after verification.
/// Construction alone does not authenticate it. Debug includes payload bytes; avoid secret logging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedMessage {
    /// Determines whether the payload is authentication, application data or control data.
    pub kind: ProtectedKind,
    /// Kind-specific bytes bounded by the decoder's configured maximum.
    pub payload: Vec<u8>,
}

impl ProtectedMessage {
    /// Copies a payload into a protected message.
    /// `kind` selects its interpretation; empty payloads are allowed here. No encryption,
    /// semantic validation or size checking occurs until the caller/encoder enforces it.
    pub fn new(kind: ProtectedKind, payload: &[u8]) -> Self {
        Self {
            kind,
            payload: payload.to_vec(),
        }
    }
}

/// Encodes content before it is passed to a Noise transport cipher.
/// Returns owned bytes with a five-byte inner header, not ciphertext. Payloads beyond
/// `max_payload_len` or u32::MAX return MessageTooLarge; a zero limit permits empty payloads.
pub fn encode_protected(message: &ProtectedMessage, max_payload_len: usize) -> Result<Vec<u8>> {
    validate_payload_len(message.payload.len(), max_payload_len)?;
    let payload_len = u32::try_from(message.payload.len()).map_err(|_| {
        RnetError::new(
            ErrorCode::MessageTooLarge,
            "protected message payload exceeds u32",
        )
    })?;
    let mut encoded = Vec::with_capacity(PROTECTED_HEADER_LEN + message.payload.len());
    encoded.push(message.kind as u8);
    encoded.extend_from_slice(&payload_len.to_be_bytes());
    encoded.extend_from_slice(&message.payload);
    Ok(encoded)
}

/// Decodes exactly one authenticated message after Noise verification succeeds.
/// The caller supplies already verified `input`; this function only checks shape/kind/length.
/// Trailing/truncated bytes or unknown kinds return ProtocolError; an excessive declared
/// payload returns MessageTooLarge before copying. Zero limit permits an empty payload.
pub fn decode_protected(input: &[u8], max_payload_len: usize) -> Result<ProtectedMessage> {
    if input.len() < PROTECTED_HEADER_LEN {
        return protocol_error("truncated protected message header");
    }
    let kind = ProtectedKind::try_from(input[0])?;
    let payload_len = u32::from_be_bytes(
        input[1..5]
            .try_into()
            .expect("fixed protected length slice"),
    ) as usize;
    validate_payload_len(payload_len, max_payload_len)?;
    let expected = PROTECTED_HEADER_LEN
        .checked_add(payload_len)
        .ok_or_else(|| {
            RnetError::new(
                ErrorCode::MessageTooLarge,
                "protected message length overflow",
            )
        })?;
    if input.len() != expected {
        return protocol_error("protected message length does not match its payload");
    }
    Ok(ProtectedMessage::new(kind, &input[PROTECTED_HEADER_LEN..]))
}

impl Record {
    /// Copies a payload into a new record.
    /// No limit or kind/epoch consistency check is performed. The driver owns those
    /// semantic checks; the encoder only enforces payload size.
    pub fn new(kind: RecordKind, epoch: u64, payload: &[u8]) -> Self {
        Self {
            kind,
            epoch,
            payload: payload.to_vec(),
        }
    }
}

/// Encodes a record after enforcing the configured payload limit.
/// Returns owned bytes with a 20-byte header; does not encrypt/authenticate or validate
/// kind/epoch consistency. Excess payload returns MessageTooLarge; zero limit allows empty.
pub fn encode_record(record: &Record, max_payload_len: usize) -> Result<Vec<u8>> {
    validate_payload_len(record.payload.len(), max_payload_len)?;
    let payload_len = u32::try_from(record.payload.len()).map_err(|_| {
        RnetError::new(
            ErrorCode::MessageTooLarge,
            "security record payload exceeds u32",
        )
    })?;
    let mut encoded = Vec::with_capacity(HEADER_LEN + record.payload.len());
    encoded.extend_from_slice(&MAGIC.to_be_bytes());
    encoded.extend_from_slice(&VERSION.to_be_bytes());
    encoded.push(record.kind as u8);
    encoded.push(0);
    encoded.extend_from_slice(&record.epoch.to_be_bytes());
    encoded.extend_from_slice(&payload_len.to_be_bytes());
    encoded.extend_from_slice(&record.payload);
    Ok(encoded)
}

/// Decodes exactly one record and rejects malformed lengths before copying the payload.
/// Unknown format/kind, reserved bits, truncation or trailing bytes return ProtocolError;
/// payload above `max_payload_len`/u32::MAX returns MessageTooLarge. A zero limit allows empty.
/// Epoch freshness and payload authenticity must be checked by the session driver.
pub fn decode_record(input: &[u8], max_payload_len: usize) -> Result<Record> {
    if input.len() < HEADER_LEN {
        return protocol_error("truncated security record header");
    }
    let magic = u32::from_be_bytes(input[0..4].try_into().expect("fixed magic slice"));
    let version = u16::from_be_bytes(input[4..6].try_into().expect("fixed version slice"));
    if magic != MAGIC || version != VERSION || input[7] != 0 {
        return protocol_error("invalid security record header");
    }
    let kind = RecordKind::try_from(input[6])?;
    let epoch = u64::from_be_bytes(input[8..16].try_into().expect("fixed epoch slice"));
    let payload_len =
        u32::from_be_bytes(input[16..20].try_into().expect("fixed length slice")) as usize;
    validate_payload_len(payload_len, max_payload_len)?;
    let expected = HEADER_LEN.checked_add(payload_len).ok_or_else(|| {
        RnetError::new(
            ErrorCode::MessageTooLarge,
            "security record length overflow",
        )
    })?;
    if input.len() != expected {
        return protocol_error("security record length does not match its payload");
    }
    Ok(Record::new(kind, epoch, &input[HEADER_LEN..]))
}

/// Identifies an authenticated transition barrier carried inside a protected record.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlKind {
    /// Proposes changing application-data mode at a new epoch.
    SwitchPropose = 1,
    /// Confirms that the receiver is ready to switch mode.
    SwitchReady = 2,
    /// Commits a previously prepared mode switch.
    SwitchCommit = 3,
    /// Confirms that the committed mode is active.
    SwitchAck = 4,
    /// Proposes deriving the next pair of Noise transport keys.
    RekeyPropose = 5,
    /// Confirms readiness to rekey.
    RekeyReady = 6,
    /// Commits a prepared rekey.
    RekeyCommit = 7,
    /// Confirms that the new keys are active.
    RekeyAck = 8,
}

impl TryFrom<u8> for ControlKind {
    type Error = RnetError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::SwitchPropose),
            2 => Ok(Self::SwitchReady),
            3 => Ok(Self::SwitchCommit),
            4 => Ok(Self::SwitchAck),
            5 => Ok(Self::RekeyPropose),
            6 => Ok(Self::RekeyReady),
            7 => Ok(Self::RekeyCommit),
            8 => Ok(Self::RekeyAck),
            _ => protocol_error("unknown security control kind"),
        }
    }
}

/// An authenticated security-mode or rekey transition message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Control {
    /// Barrier operation represented by this message.
    pub kind: ControlKind,
    /// Target epoch. Epoch zero is reserved for the handshake.
    pub epoch: u64,
    /// Target data mode for `SwitchPropose`; absent for the remaining barriers.
    pub mode: Option<SecurityMode>,
}

impl Control {
    /// Constructs a control carrying `mode`; caller must use SwitchPropose and a nonzero epoch.
    /// No validation is performed here; invalid combinations will fail peer decoding.
    pub fn switch(kind: ControlKind, epoch: u64, mode: SecurityMode) -> Self {
        Self {
            kind,
            epoch,
            mode: Some(mode),
        }
    }

    /// Creates a transition barrier that carries no mode.
    /// Caller supplies a nonzero `epoch` and a kind other than SwitchPropose; not validated here.
    pub fn barrier(kind: ControlKind, epoch: u64) -> Self {
        Self {
            kind,
            epoch,
            mode: None,
        }
    }
}

/// Encodes a fixed-size control message for encryption inside a protected record.
/// Serializes `control` without validation or encryption. Caller must supply a nonzero epoch
/// and mode only for SwitchPropose; use the authenticated channel for transmission.
pub fn encode_control(control: Control) -> [u8; CONTROL_LEN] {
    let mut encoded = [0; CONTROL_LEN];
    encoded[0] = control.kind as u8;
    encoded[1] = control.mode.map_or(0, |mode| mode as u8);
    encoded[2..].copy_from_slice(&control.epoch.to_be_bytes());
    encoded
}

/// Decodes a transition control and validates its kind, mode and nonzero epoch.
/// `input` must already be authenticated and contain exactly ten bytes. Invalid shapes/values
/// return ProtocolError; session role, expected epoch and barrier order are checked elsewhere.
pub fn decode_control(input: &[u8]) -> Result<Control> {
    if input.len() != CONTROL_LEN {
        return protocol_error("invalid security control length");
    }
    let kind = ControlKind::try_from(input[0])?;
    let epoch = u64::from_be_bytes(input[2..].try_into().expect("fixed control epoch slice"));
    if epoch == 0 {
        return protocol_error("security transition epoch must be nonzero");
    }
    let mode = match kind {
        ControlKind::SwitchPropose => Some(SecurityMode::try_from(input[1])?),
        _ if input[1] == 0 => None,
        _ => return protocol_error("unexpected mode in security transition barrier"),
    };
    Ok(Control { kind, epoch, mode })
}

fn validate_payload_len(payload_len: usize, max_payload_len: usize) -> Result<()> {
    if payload_len > max_payload_len || payload_len > u32::MAX as usize {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "security record payload exceeds configured limit",
        ));
    }
    Ok(())
}

fn protocol_error<T>(message: &str) -> Result<T> {
    Err(RnetError::new(ErrorCode::ProtocolError, message))
}
