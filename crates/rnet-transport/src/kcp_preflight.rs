use crate::cookie::COOKIE_LEN;
use ring::rand::{SecureRandom, SystemRandom};
use rnet_core::{ErrorCode, Result, RnetError};

const MAGIC: &[u8; 4] = b"RNK2";
const HELLO: u8 = 1;
const CHALLENGE: u8 = 2;
const HEADER_LEN: usize = 9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Preflight<'a> {
    Hello { conv: u32, cookie: &'a [u8] },
    Challenge { conv: u32, cookie: &'a [u8] },
}

pub(crate) fn random_conv() -> Result<u32> {
    let mut bytes = [0_u8; 4];
    SystemRandom::new().fill(&mut bytes).map_err(|_| {
        RnetError::new(
            ErrorCode::CryptoError,
            "operating-system randomness unavailable for KCP conversation",
        )
    })?;
    let conv = u32::from_be_bytes(bytes);
    Ok(if conv == 0 { 1 } else { conv })
}

pub(crate) fn encode_hello(conv: u32, cookie: &[u8]) -> Vec<u8> {
    encode(HELLO, conv, cookie)
}

pub(crate) fn encode_challenge(conv: u32, cookie: &[u8]) -> Vec<u8> {
    encode(CHALLENGE, conv, cookie)
}

fn encode(kind: u8, conv: u32, cookie: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(HEADER_LEN + cookie.len());
    packet.extend_from_slice(MAGIC);
    packet.push(kind);
    packet.extend_from_slice(&conv.to_be_bytes());
    packet.extend_from_slice(cookie);
    packet
}

pub(crate) fn decode(packet: &[u8]) -> Option<Preflight<'_>> {
    let payload = packet.strip_prefix(MAGIC)?;
    if payload.len() < 5 {
        return None;
    }
    let kind = payload[0];
    let conv = u32::from_be_bytes(payload[1..5].try_into().ok()?);
    if conv == 0 {
        return None;
    }
    let cookie = &payload[5..];
    match (kind, cookie.len()) {
        (HELLO, 0 | COOKIE_LEN) => Some(Preflight::Hello { conv, cookie }),
        (CHALLENGE, COOKIE_LEN) => Some(Preflight::Challenge { conv, cookie }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode_challenge, encode_hello, Preflight};

    #[test]
    fn preflight_messages_are_distinct_and_length_checked() {
        let cookie = [7_u8; 40];
        let conv = 0x1234_5678;
        assert_eq!(
            decode(&encode_hello(conv, &[])),
            Some(Preflight::Hello { conv, cookie: &[] })
        );
        assert_eq!(
            decode(&encode_hello(conv, &cookie)),
            Some(Preflight::Hello {
                conv,
                cookie: cookie.as_slice()
            })
        );
        assert_eq!(
            decode(&encode_challenge(conv, &cookie)),
            Some(Preflight::Challenge {
                conv,
                cookie: cookie.as_slice()
            })
        );
        assert_eq!(decode(b"RNKP\x01short"), None);
        assert_eq!(decode(b"not-preflight"), None);
    }
}
