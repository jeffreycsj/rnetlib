//! Game join metadata carried inside the Noise-protected client handshake payload.

use crate::config::{GameProtocol, GameProtocolRange};
use rnet_core::{ErrorCode, Result, RnetError};

// The old RGJ3 marker denoted a v2 resume join, so v3 uses disjoint markers
// for ordinary and resumed joins. Old clients fail before business authorization.
const MAGIC: &[u8; 4] = b"RGV3";
const RESUME_MAGIC: &[u8; 4] = b"RGR3";
const RANGE_MAGIC: &[u8; 4] = b"RGV4";
const RANGE_RESUME_MAGIC: &[u8; 4] = b"RGR4";
pub(crate) const HEADER_LEN: usize = 34;
const RESUME_HEADER_LEN: usize = 36;
const RANGE_HEADER_LEN: usize = 54;
const RANGE_RESUME_HEADER_LEN: usize = 56;

#[derive(Debug)]
pub(crate) struct RangeJoinRequest<'a> {
    pub protocol: GameProtocolRange,
    pub nonce: [u8; 16],
    pub ticket: &'a [u8],
    pub resume_ticket: Option<&'a [u8]>,
}

/// The server selects the highest common version, never a client-preferred downgrade.
pub(crate) fn select_version(client: GameProtocolRange, server: GameProtocolRange) -> Option<u32> {
    if !client.is_valid() || !server.is_valid() || client.protocol_id != server.protocol_id {
        return None;
    }
    let lower = client.min_version.max(server.min_version);
    let upper = client.max_version.min(server.max_version);
    (lower <= upper).then_some(upper)
}

/// Explicit wire-v4 range joins cannot be interpreted by the exact-version v3 decoder.
pub(crate) fn encode_range(
    protocol: GameProtocolRange,
    nonce: [u8; 16],
    ticket: &[u8],
    maximum_len: usize,
) -> Result<Vec<u8>> {
    encode_range_inner(protocol, nonce, ticket, None, maximum_len)
}

pub(crate) fn encode_range_resume(
    protocol: GameProtocolRange,
    nonce: [u8; 16],
    ticket: &[u8],
    resume_ticket: &[u8],
    maximum_len: usize,
) -> Result<Vec<u8>> {
    if resume_ticket.is_empty() {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "resume ticket is empty",
        ));
    }
    encode_range_inner(protocol, nonce, ticket, Some(resume_ticket), maximum_len)
}

fn encode_range_inner(
    protocol: GameProtocolRange,
    nonce: [u8; 16],
    ticket: &[u8],
    resume_ticket: Option<&[u8]>,
    maximum_len: usize,
) -> Result<Vec<u8>> {
    if !protocol.is_valid() || nonce == [0; 16] {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "invalid game protocol range or negotiation nonce",
        ));
    }
    let header_len = if resume_ticket.is_some() {
        RANGE_RESUME_HEADER_LEN
    } else {
        RANGE_HEADER_LEN
    };
    let total = header_len
        .checked_add(ticket.len())
        .and_then(|len| len.checked_add(resume_ticket.map_or(0, <[u8]>::len)))
        .ok_or_else(|| RnetError::new(ErrorCode::MessageTooLarge, "range join length overflow"))?;
    if ticket.len() > u16::MAX as usize
        || resume_ticket.is_some_and(|ticket| ticket.len() > u16::MAX as usize)
        || total > maximum_len
    {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "range join exceeds the configured handshake limit",
        ));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(if resume_ticket.is_some() {
        RANGE_RESUME_MAGIC
    } else {
        RANGE_MAGIC
    });
    out.extend_from_slice(&protocol.protocol_id.to_be_bytes());
    out.extend_from_slice(&protocol.min_version.to_be_bytes());
    out.extend_from_slice(&protocol.max_version.to_be_bytes());
    out.extend_from_slice(&protocol.build_id.to_be_bytes());
    out.extend_from_slice(&protocol.capabilities.to_be_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&(ticket.len() as u16).to_be_bytes());
    if let Some(resume_ticket) = resume_ticket {
        out.extend_from_slice(&(resume_ticket.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(ticket);
    if let Some(resume_ticket) = resume_ticket {
        out.extend_from_slice(resume_ticket);
    }
    Ok(out)
}

pub(crate) fn decode_range(input: &[u8]) -> Result<RangeJoinRequest<'_>> {
    if input.len() < RANGE_HEADER_LEN
        || (&input[..4] != RANGE_MAGIC && &input[..4] != RANGE_RESUME_MAGIC)
    {
        return Err(invalid_join());
    }
    let protocol = GameProtocolRange {
        protocol_id: u64::from_be_bytes(input[4..12].try_into().expect("fixed protocol ID")),
        min_version: u32::from_be_bytes(input[12..16].try_into().expect("fixed minimum")),
        max_version: u32::from_be_bytes(input[16..20].try_into().expect("fixed maximum")),
        build_id: u64::from_be_bytes(input[20..28].try_into().expect("fixed build ID")),
        capabilities: u64::from_be_bytes(input[28..36].try_into().expect("fixed capabilities")),
    };
    let nonce: [u8; 16] = input[36..52].try_into().expect("fixed nonce");
    let ticket_len = usize::from(u16::from_be_bytes([input[52], input[53]]));
    if !protocol.is_valid() || nonce == [0; 16] {
        return Err(invalid_join());
    }
    let resume = &input[..4] == RANGE_RESUME_MAGIC;
    let (header_len, resume_len) = if resume {
        if input.len() < RANGE_RESUME_HEADER_LEN {
            return Err(invalid_join());
        }
        (
            RANGE_RESUME_HEADER_LEN,
            usize::from(u16::from_be_bytes([input[54], input[55]])),
        )
    } else {
        (RANGE_HEADER_LEN, 0)
    };
    if (resume && resume_len == 0) || input.len() != header_len + ticket_len + resume_len {
        return Err(invalid_join());
    }
    let ticket_end = header_len + ticket_len;
    Ok(RangeJoinRequest {
        protocol,
        nonce,
        ticket: &input[header_len..ticket_end],
        resume_ticket: resume.then_some(&input[ticket_end..]),
    })
}

pub(crate) struct JoinRequest<'a> {
    pub protocol: GameProtocol,
    pub ticket: &'a [u8],
    pub resume_ticket: Option<&'a [u8]>,
}

impl std::fmt::Debug for JoinRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JoinRequest")
            .field("protocol", &self.protocol)
            .field("ticket_len", &self.ticket.len())
            .field("resume_ticket_len", &self.resume_ticket.map(<[u8]>::len))
            .finish()
    }
}

/// A distinct join magic prevents legacy join decoders from treating resume bytes as login data.
pub(crate) fn encode_resume(
    protocol: GameProtocol,
    ticket: &[u8],
    resume_ticket: &[u8],
    maximum_len: usize,
) -> Result<Vec<u8>> {
    if !protocol.is_valid() || resume_ticket.is_empty() {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "invalid resume join metadata",
        ));
    }
    let total = RESUME_HEADER_LEN
        .checked_add(ticket.len())
        .and_then(|len| len.checked_add(resume_ticket.len()))
        .ok_or_else(|| RnetError::new(ErrorCode::MessageTooLarge, "resume join length overflow"))?;
    if ticket.len() > u16::MAX as usize
        || resume_ticket.len() > u16::MAX as usize
        || total > maximum_len
    {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "resume join exceeds the configured handshake limit",
        ));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(RESUME_MAGIC);
    out.extend_from_slice(&protocol.protocol_id.to_be_bytes());
    out.extend_from_slice(&protocol.version.to_be_bytes());
    out.extend_from_slice(&protocol.build_id.to_be_bytes());
    out.extend_from_slice(&protocol.capabilities.to_be_bytes());
    out.extend_from_slice(&(ticket.len() as u16).to_be_bytes());
    out.extend_from_slice(&(resume_ticket.len() as u16).to_be_bytes());
    out.extend_from_slice(ticket);
    out.extend_from_slice(resume_ticket);
    Ok(out)
}

/// Encodes a bounded ticket without exposing protocol metadata to business authorization.
pub(crate) fn encode(protocol: GameProtocol, ticket: &[u8], maximum_len: usize) -> Result<Vec<u8>> {
    if !protocol.is_valid() {
        return Err(RnetError::new(
            ErrorCode::InvalidArgument,
            "game protocol ID and version must be positive",
        ));
    }
    let total = HEADER_LEN
        .checked_add(ticket.len())
        .ok_or_else(|| RnetError::new(ErrorCode::MessageTooLarge, "game join length overflow"))?;
    if ticket.len() > u16::MAX as usize || total > maximum_len {
        return Err(RnetError::new(
            ErrorCode::MessageTooLarge,
            "game join exceeds the configured handshake limit",
        ));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&protocol.protocol_id.to_be_bytes());
    out.extend_from_slice(&protocol.version.to_be_bytes());
    out.extend_from_slice(&protocol.build_id.to_be_bytes());
    out.extend_from_slice(&protocol.capabilities.to_be_bytes());
    out.extend_from_slice(&(ticket.len() as u16).to_be_bytes());
    out.extend_from_slice(ticket);
    Ok(out)
}

/// Rejects truncated, trailing, or invalid metadata before handing the ticket to game code.
pub(crate) fn decode(input: &[u8]) -> Result<JoinRequest<'_>> {
    if input.len() < HEADER_LEN || (&input[..4] != MAGIC && &input[..4] != RESUME_MAGIC) {
        return Err(invalid_join());
    }
    let protocol = GameProtocol {
        protocol_id: u64::from_be_bytes(input[4..12].try_into().expect("fixed protocol ID")),
        version: u32::from_be_bytes(input[12..16].try_into().expect("fixed version")),
        build_id: u64::from_be_bytes(input[16..24].try_into().expect("fixed build ID")),
        capabilities: u64::from_be_bytes(input[24..32].try_into().expect("fixed capabilities")),
    };
    let ticket_len = usize::from(u16::from_be_bytes([input[32], input[33]]));
    if !protocol.is_valid() {
        return Err(invalid_join());
    }
    if &input[..4] == MAGIC {
        if input.len() != HEADER_LEN + ticket_len {
            return Err(invalid_join());
        }
        return Ok(JoinRequest {
            protocol,
            ticket: &input[HEADER_LEN..],
            resume_ticket: None,
        });
    }
    if input.len() < RESUME_HEADER_LEN {
        return Err(invalid_join());
    }
    let resume_len = usize::from(u16::from_be_bytes([input[34], input[35]]));
    if resume_len == 0 || input.len() != RESUME_HEADER_LEN + ticket_len + resume_len {
        return Err(invalid_join());
    }
    let ticket_end = RESUME_HEADER_LEN + ticket_len;
    Ok(JoinRequest {
        protocol,
        ticket: &input[RESUME_HEADER_LEN..ticket_end],
        resume_ticket: Some(&input[ticket_end..]),
    })
}

fn invalid_join() -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, "malformed game join metadata")
}
