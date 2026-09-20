//! Private game-message envelope carried inside the existing RNet frame body.
//!
//! The outer frame keeps its compatibility `msg_type` and `stream_id` fields at zero. Application
//! bytes begin after this header and remain opaque, so business routing stays in the caller's
//! protobuf or other schema while network controls cannot be confused with application data.

use bytes::{BufMut, Bytes, BytesMut};
use rnet_core::{ErrorCode, Result, RnetError};

pub(crate) const HEADER_LEN: usize = 12;
const APPLICATION: u8 = 0;
const HEARTBEAT: u8 = 1;
const CLOCK_SYNC: u8 = 2;
const RESUME: u8 = 3;
const PROTOCOL: u8 = 4;
const HAS_SEQUENCE: u8 = 1 << 0;
const HAS_TICK: u8 = 1 << 1;
const HAS_DATAGRAM_SEQUENCE: u8 = 1 << 2;
const SUPPORTED_FLAGS: u8 = HAS_SEQUENCE | HAS_TICK | HAS_DATAGRAM_SEQUENCE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControlKind {
    Heartbeat,
    ClockSync,
    Resume,
    Protocol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DecodedEnvelope {
    Application {
        sequence: Option<u32>,
        tick: Option<u32>,
        datagram_sequence: Option<u32>,
        payload: Bytes,
    },
    Control {
        kind: ControlKind,
        payload: Bytes,
    },
}

/// Encodes opaque business bytes with optional network-owned ordering metadata.
pub(crate) fn encode_application(
    payload: &[u8],
    sequence: Option<u32>,
    tick: Option<u32>,
    maximum_len: usize,
) -> Result<Bytes> {
    let mut flags = 0;
    if sequence.is_some() {
        flags |= HAS_SEQUENCE;
    }
    if tick.is_some() {
        flags |= HAS_TICK;
    }
    encode(
        APPLICATION,
        flags,
        sequence.unwrap_or_default(),
        tick.unwrap_or_default(),
        None,
        payload,
        maximum_len,
    )
}

/// Adds a library-owned sequence to UDP business datagrams without consuming application metadata.
pub(crate) fn encode_udp_application(
    payload: &[u8],
    sequence: Option<u32>,
    tick: Option<u32>,
    datagram_sequence: u32,
    maximum_len: usize,
) -> Result<Bytes> {
    let mut flags = HAS_DATAGRAM_SEQUENCE;
    if sequence.is_some() {
        flags |= HAS_SEQUENCE;
    }
    if tick.is_some() {
        flags |= HAS_TICK;
    }
    encode(
        APPLICATION,
        flags,
        sequence.unwrap_or_default(),
        tick.unwrap_or_default(),
        Some(datagram_sequence),
        payload,
        maximum_len,
    )
}

/// Encodes a library control without exposing its discriminator to the application API.
#[allow(dead_code)] // Used by heartbeat/recovery tasks after the application path is established.
pub(crate) fn encode_control(
    kind: ControlKind,
    payload: &[u8],
    maximum_len: usize,
) -> Result<Bytes> {
    let wire_kind = match kind {
        ControlKind::Heartbeat => HEARTBEAT,
        ControlKind::ClockSync => CLOCK_SYNC,
        ControlKind::Resume => RESUME,
        ControlKind::Protocol => PROTOCOL,
    };
    encode(wire_kind, 0, 0, 0, None, payload, maximum_len)
}

/// Validates the complete untrusted envelope before returning a payload view owned by the caller.
pub(crate) fn decode(input: &[u8], maximum_len: usize) -> Result<DecodedEnvelope> {
    if input.len() > maximum_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "game envelope exceeds the configured limit",
        ));
    }
    if input.len() < HEADER_LEN {
        return Err(protocol_error("game envelope header is truncated"));
    }
    let kind = input[0];
    let flags = input[1];
    if flags & !SUPPORTED_FLAGS != 0 {
        return Err(protocol_error("game envelope contains unsupported flags"));
    }
    let header_len = usize::from(u16::from_be_bytes([input[2], input[3]]));
    if !(HEADER_LEN..=input.len()).contains(&header_len) {
        return Err(protocol_error("game envelope header length is invalid"));
    }
    let sequence = u32::from_be_bytes(input[4..8].try_into().expect("fixed sequence field"));
    let tick = u32::from_be_bytes(input[8..12].try_into().expect("fixed tick field"));
    let datagram_sequence = if flags & HAS_DATAGRAM_SEQUENCE != 0 {
        if header_len < HEADER_LEN + 4 || kind != APPLICATION {
            return Err(protocol_error("UDP sequence extension is invalid"));
        }
        Some(u32::from_be_bytes(
            input[HEADER_LEN..HEADER_LEN + 4]
                .try_into()
                .expect("validated UDP sequence extension"),
        ))
    } else {
        None
    };
    let payload = Bytes::copy_from_slice(&input[header_len..]);

    if kind == APPLICATION {
        if flags & HAS_SEQUENCE == 0 && sequence != 0 {
            return Err(protocol_error("game sequence is set without its flag"));
        }
        if flags & HAS_TICK == 0 && tick != 0 {
            return Err(protocol_error("game tick is set without its flag"));
        }
        return Ok(DecodedEnvelope::Application {
            sequence: (flags & HAS_SEQUENCE != 0).then_some(sequence),
            tick: (flags & HAS_TICK != 0).then_some(tick),
            datagram_sequence,
            payload,
        });
    }

    // Controls never accept application metadata. This keeps their signed wire representation
    // canonical and prevents peers from smuggling unvalidated sequencing semantics into them.
    if flags != 0 || sequence != 0 || tick != 0 {
        return Err(protocol_error(
            "game control contains application-only metadata",
        ));
    }
    let kind = match kind {
        HEARTBEAT => ControlKind::Heartbeat,
        CLOCK_SYNC => ControlKind::ClockSync,
        RESUME => ControlKind::Resume,
        PROTOCOL => ControlKind::Protocol,
        _ => return Err(protocol_error("game envelope kind is unsupported")),
    };
    Ok(DecodedEnvelope::Control { kind, payload })
}

fn encode(
    kind: u8,
    flags: u8,
    sequence: u32,
    tick: u32,
    datagram_sequence: Option<u32>,
    payload: &[u8],
    maximum_len: usize,
) -> Result<Bytes> {
    let header_len = HEADER_LEN + 4 * usize::from(datagram_sequence.is_some());
    let encoded_len = header_len.checked_add(payload.len()).ok_or_else(|| {
        RnetError::new(ErrorCode::MessageTooLarge, "game envelope length overflow")
    })?;
    if encoded_len > maximum_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "game envelope exceeds the configured limit",
        ));
    }
    let mut encoded = BytesMut::with_capacity(encoded_len);
    encoded.put_u8(kind);
    encoded.put_u8(flags);
    encoded.put_u16(header_len as u16);
    encoded.put_u32(sequence);
    encoded.put_u32(tick);
    if let Some(datagram_sequence) = datagram_sequence {
        encoded.put_u32(datagram_sequence);
    }
    encoded.extend_from_slice(payload);
    Ok(encoded.freeze())
}

fn protocol_error(message: &'static str) -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, message)
}
