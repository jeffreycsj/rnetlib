//! Current logical framing and security-record serialization, without network I/O.
//!
//! Parsers validate structure, not peer identity. Only decrypted/authenticated input
//! belongs in protected-message/control decoding. Business versions are negotiated
//! above this layer; wire identifiers are exact-match and never downgrade.

#![deny(missing_docs)]

pub mod control;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message;
use rnet_core::{ErrorCode, Result, RnetError};

/// Big-endian logical-frame magic (`RNET`); not a business message type.
pub const MAGIC: u32 = 0x524e_4554;
/// Only this layout is accepted; older SDK frames are never reinterpreted or downgraded.
pub const VERSION: u16 = 2;
/// Encoded logical-frame header size in bytes, excluded from the body limit.
pub const HEADER_LEN: usize = 20;
/// Supported flag bitmask. Currently zero: every nonzero flag is rejected.
pub const SUPPORTED_FLAGS: u16 = 0;

/// Decoded logical-frame metadata. Constructing this struct does not validate its fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHeader {
    /// Must equal VERSION on the wire; independent of negotiated business protocol versions.
    pub version: u16,
    /// Reserved bitmask; only SUPPORTED_FLAGS bits are accepted.
    pub flags: u16,
    /// Declared body byte count, excluding HEADER_LEN; checked before waiting for the body.
    pub body_len: u32,
    /// Opaque caller correlation, with zero meaning unspecified; not a routing or dedup key.
    pub correlation_id: u64,
}

/// One complete logical frame; the decoder ensures header/body length consistency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    /// Validated wire metadata when returned by a decoder; public mutation is not revalidated.
    pub header: FrameHeader,
    /// Owned, reference-counted body bytes, possibly sharing a parsed allocation.
    /// No decryption, business decoding or secret zeroization is performed by this type.
    pub body: Bytes,
}

/// Incremental stream parser; keeps a partial frame between pushes.
/// The body limit is per frame, not a total input/allocation budget. Bound input chunks
/// at the I/O layer. On parse error discard this codec; no resynchronization is attempted.
///
/// ```
/// use rnet_protocol::{encode_frame, FrameCodec, HEADER_LEN};
/// let wire = encode_frame(42, 0, b"hello", 128)?;
/// let mut parser = FrameCodec::new(128)?;
/// assert!(parser.push(&wire[..HEADER_LEN - 1])?.is_empty());
/// let frames = parser.push(&wire[HEADER_LEN - 1..])?;
/// assert_eq!(frames.len(), 1);
/// assert_eq!(frames[0].header.correlation_id, 42);
/// assert_eq!(frames[0].body.as_ref(), b"hello");
/// # Ok::<(), rnet_core::RnetError>(())
/// ```
#[derive(Debug)]
pub struct FrameCodec {
    max_body_len: usize,
    buffer: BytesMut,
}

impl FrameCodec {
    /// Creates an empty parser with an inclusive per-body byte limit in 1..=u32::MAX.
    /// Invalid limits return InvalidArgument; header bytes are additional overhead.
    pub fn new(max_body_len: usize) -> Result<Self> {
        if max_body_len == 0 || max_body_len > u32::MAX as usize {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "maximum body length must be between 1 and u32::MAX",
            ));
        }
        Ok(Self {
            max_body_len,
            buffer: BytesMut::with_capacity(HEADER_LEN),
        })
    }

    /// Copies `input` and emits all complete frames; incomplete trailing bytes remain buffered.
    /// Empty input is allowed. Invalid headers/flags return ProtocolError; oversized bodies
    /// return MessageTooLarge. An error may occur after earlier frames have been consumed,
    /// so the operation is not transactional and those frames are not returned on failure.
    /// Input is appended before parsing: callers must independently bound the supplied slice.
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<Frame>> {
        self.buffer.extend_from_slice(input);
        let mut frames = Vec::new();
        loop {
            if self.buffer.len() < HEADER_LEN {
                break;
            }
            let header = parse_header(&self.buffer[..HEADER_LEN], self.max_body_len)?;
            let frame_len = HEADER_LEN
                .checked_add(header.body_len as usize)
                .ok_or_else(|| {
                    RnetError::new(ErrorCode::MessageTooLarge, "frame length overflow")
                })?;
            if self.buffer.len() < frame_len {
                break;
            }

            let mut raw = self.buffer.split_to(frame_len);
            raw.advance(HEADER_LEN);
            frames.push(Frame {
                header,
                body: raw.freeze(),
            });
        }
        Ok(frames)
    }
}

/// Copies `body` into one current-format frame carrying opaque `correlation_id` (zero absent).
/// `flags` must be zero and `max_body_len` must be 1..=u32::MAX; an empty body is valid.
/// Returns InvalidArgument for invalid limits, ProtocolError for flags, or MessageTooLarge
/// for an oversized body. Output includes HEADER_LEN additional bytes; no encryption is applied.
pub fn encode_frame(
    correlation_id: u64,
    flags: u16,
    body: &[u8],
    max_body_len: usize,
) -> Result<Bytes> {
    if max_body_len == 0 || max_body_len > u32::MAX as usize {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "maximum body length must be between 1 and u32::MAX",
        ));
    }
    validate_flags(flags)?;
    if body.len() > max_body_len || body.len() > u32::MAX as usize {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "message exceeds configured body limit",
        ));
    }

    let mut encoded = BytesMut::with_capacity(HEADER_LEN + body.len());
    encoded.put_u32(MAGIC);
    encoded.put_u16(VERSION);
    encoded.put_u16(flags);
    encoded.put_u32(body.len() as u32);
    encoded.put_u64(correlation_id);
    encoded.extend_from_slice(body);
    Ok(encoded.freeze())
}

/// Decodes exactly one complete frame from `input`, copying its body.
/// `max_body_len` is an inclusive byte ceiling; zero permits only an empty body.
/// Rejects truncation, trailing bytes, version/flags mismatches with ProtocolError and
/// oversized declared bodies with MessageTooLarge. This is framing, not authentication.
pub fn decode_datagram(input: &[u8], max_body_len: usize) -> Result<Frame> {
    if input.len() < HEADER_LEN {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "datagram does not contain a complete header",
        ));
    }
    let header = parse_header(&input[..HEADER_LEN], max_body_len)?;
    let expected = HEADER_LEN + header.body_len as usize;
    if input.len() != expected {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "datagram must contain exactly one complete frame",
        ));
    }
    Ok(Frame {
        header,
        body: Bytes::copy_from_slice(&input[HEADER_LEN..]),
    })
}

fn parse_header(input: &[u8], max_body_len: usize) -> Result<FrameHeader> {
    let magic = u32::from_be_bytes(input[0..4].try_into().expect("fixed header slice"));
    if magic != MAGIC {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "invalid frame magic",
        ));
    }
    let version = u16::from_be_bytes(input[4..6].try_into().expect("fixed header slice"));
    if version != VERSION {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "unsupported frame version",
        ));
    }
    let flags = u16::from_be_bytes(input[6..8].try_into().expect("fixed header slice"));
    validate_flags(flags)?;
    let body_len = u32::from_be_bytes(input[8..12].try_into().expect("fixed header slice"));
    if body_len as usize > max_body_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "declared body exceeds configured limit",
        ));
    }
    Ok(FrameHeader {
        version,
        flags,
        body_len,
        correlation_id: u64::from_be_bytes(input[12..20].try_into().expect("fixed header slice")),
    })
}

fn validate_flags(flags: u16) -> Result<()> {
    if flags & !SUPPORTED_FLAGS != 0 {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "frame contains unsupported flags",
        ));
    }
    Ok(())
}

/// Encodes a Protobuf message without a network frame/envelope into owned bytes.
/// `max_encoded_len` must be positive (InvalidArgument); oversized output returns
/// MessageTooLarge and encoder errors return ProtocolError. Empty encoded messages are valid.
pub fn encode_protobuf<M: Message>(message: &M, max_encoded_len: usize) -> Result<Bytes> {
    if max_encoded_len == 0 {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "maximum Protobuf length must be positive",
        ));
    }
    let encoded_len = message.encoded_len();
    if encoded_len > max_encoded_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "encoded Protobuf message exceeds configured limit",
        ));
    }
    let mut bytes = BytesMut::with_capacity(encoded_len);
    message.encode(&mut bytes).map_err(|error| {
        RnetError::new(
            ErrorCode::ProtocolError,
            format!("failed to encode Protobuf message: {error}"),
        )
    })?;
    Ok(bytes.freeze())
}

/// Decodes Protobuf from `bytes` without authenticating or parsing network framing.
/// A zero limit returns InvalidArgument; input beyond `max_encoded_len` returns
/// MessageTooLarge and decoding failures return ProtocolError. The wire-byte limit
/// is not a bound on allocations made while decoding nested/repeated business fields.
pub fn decode_protobuf<M: Message + Default>(bytes: &[u8], max_encoded_len: usize) -> Result<M> {
    if max_encoded_len == 0 {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "maximum Protobuf length must be positive",
        ));
    }
    if bytes.len() > max_encoded_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "Protobuf input exceeds configured limit",
        ));
    }
    M::decode(bytes).map_err(|error| {
        RnetError::new(
            ErrorCode::ProtocolError,
            format!("failed to decode Protobuf message: {error}"),
        )
    })
}
