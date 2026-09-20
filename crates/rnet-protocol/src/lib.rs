//! RNet wire framing and incremental parsing.

pub mod control;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message;
use rnet_core::{ErrorCode, Result, RnetError};

pub const MAGIC: u32 = 0x524e_4554;
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 28;
pub const SUPPORTED_FLAGS: u16 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameHeader {
    pub version: u16,
    pub flags: u16,
    pub msg_type: u32,
    pub stream_id: u32,
    pub body_len: u32,
    pub request_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub header: FrameHeader,
    pub body: Bytes,
}

#[derive(Debug)]
pub struct FrameCodec {
    max_body_len: usize,
    buffer: BytesMut,
}

impl FrameCodec {
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

pub fn encode_frame(
    msg_type: u32,
    stream_id: u32,
    request_id: u64,
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
    encoded.put_u32(msg_type);
    encoded.put_u32(stream_id);
    encoded.put_u32(body.len() as u32);
    encoded.put_u64(request_id);
    encoded.extend_from_slice(body);
    Ok(encoded.freeze())
}

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
    let body_len = u32::from_be_bytes(input[16..20].try_into().expect("fixed header slice"));
    if body_len as usize > max_body_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "declared body exceeds configured limit",
        ));
    }
    Ok(FrameHeader {
        version,
        flags,
        msg_type: u32::from_be_bytes(input[8..12].try_into().expect("fixed header slice")),
        stream_id: u32::from_be_bytes(input[12..16].try_into().expect("fixed header slice")),
        body_len,
        request_id: u64::from_be_bytes(input[20..28].try_into().expect("fixed header slice")),
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
