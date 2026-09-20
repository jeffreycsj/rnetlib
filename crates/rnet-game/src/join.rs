//! Game join metadata carried inside the Noise-protected client handshake payload.

use crate::config::GameProtocol;
use rnet_core::{ErrorCode, Result, RnetError};

const MAGIC: &[u8; 4] = b"RGJ2";
const RESUME_MAGIC: &[u8; 4] = b"RGJ3";
pub(crate) const HEADER_LEN: usize = 34;
const RESUME_HEADER_LEN: usize = 36;

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
