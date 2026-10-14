//! Game join metadata carried inside the Noise-protected client handshake payload.

use crate::config::GameProtocol;
use rnet_core::{ErrorCode, Result, RnetError};

// Only the current SDK wire is accepted, independently of application version ranges.
const RANGE_MAGIC: &[u8; 4] = b"RGV4";
const RANGE_RESUME_MAGIC: &[u8; 4] = b"RGR4";
pub(crate) const HEADER_LEN: usize = 54;
const RANGE_RESUME_HEADER_LEN: usize = 56;

pub(crate) struct RangeJoinRequest<'a> {
    pub protocol: GameProtocol,
    pub nonce: [u8; 16],
    pub ticket: &'a [u8],
    pub resume_ticket: Option<&'a [u8]>,
}

/// The server selects the highest common version, never a client-preferred downgrade.
pub(crate) fn select_version(client: GameProtocol, server: GameProtocol) -> Option<u32> {
    if !client.is_valid() || !server.is_valid() || client.protocol_id != server.protocol_id {
        return None;
    }
    let lower = client.min_version.max(server.min_version);
    let upper = client.max_version.min(server.max_version);
    (lower <= upper).then_some(upper)
}

/// Carries a bounded credential and nonce; an exact application version uses equal bounds.
pub(crate) fn encode_range(
    protocol: GameProtocol,
    nonce: [u8; 16],
    ticket: &[u8],
    maximum_len: usize,
) -> Result<Vec<u8>> {
    encode_range_inner(protocol, nonce, ticket, None, maximum_len)
}

pub(crate) fn encode_range_resume(
    protocol: GameProtocol,
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
    protocol: GameProtocol,
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
        HEADER_LEN
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
    if input.len() < HEADER_LEN || (&input[..4] != RANGE_MAGIC && &input[..4] != RANGE_RESUME_MAGIC)
    {
        return Err(invalid_join());
    }
    let protocol = GameProtocol {
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
        (HEADER_LEN, 0)
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

impl std::fmt::Debug for RangeJoinRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RangeJoinRequest")
            .field("protocol", &self.protocol)
            .field("ticket_len", &self.ticket.len())
            .field("resume_ticket_len", &self.resume_ticket.map(<[u8]>::len))
            .finish()
    }
}

fn invalid_join() -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, "malformed game join metadata")
}
