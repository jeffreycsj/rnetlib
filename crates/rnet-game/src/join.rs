//! Game join metadata carried inside the Noise-protected client handshake payload.

use crate::config::GameProtocol;
use rnet_core::{ErrorCode, Result, RnetError};

const MAGIC: &[u8; 4] = b"RGJ2";
pub(crate) const HEADER_LEN: usize = 34;

#[derive(Debug)]
pub(crate) struct JoinRequest<'a> {
    pub protocol: GameProtocol,
    pub ticket: &'a [u8],
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
    if input.len() < HEADER_LEN || &input[..4] != MAGIC {
        return Err(invalid_join());
    }
    let protocol = GameProtocol {
        protocol_id: u64::from_be_bytes(input[4..12].try_into().expect("fixed protocol ID")),
        version: u32::from_be_bytes(input[12..16].try_into().expect("fixed version")),
        build_id: u64::from_be_bytes(input[16..24].try_into().expect("fixed build ID")),
        capabilities: u64::from_be_bytes(input[24..32].try_into().expect("fixed capabilities")),
    };
    let ticket_len = usize::from(u16::from_be_bytes([input[32], input[33]]));
    if !protocol.is_valid() || input.len() != HEADER_LEN + ticket_len {
        return Err(invalid_join());
    }
    Ok(JoinRequest {
        protocol,
        ticket: &input[HEADER_LEN..],
    })
}

fn invalid_join() -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, "malformed game join metadata")
}
