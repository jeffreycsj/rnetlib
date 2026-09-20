use crate::cookie::COOKIE_LEN;

const MAGIC: &[u8; 4] = b"RNKP";
const HELLO: u8 = 1;
const CHALLENGE: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Preflight<'a> {
    Hello(&'a [u8]),
    Challenge(&'a [u8]),
}

pub(crate) fn encode_hello(cookie: &[u8]) -> Vec<u8> {
    encode(HELLO, cookie)
}

pub(crate) fn encode_challenge(cookie: &[u8]) -> Vec<u8> {
    encode(CHALLENGE, cookie)
}

fn encode(kind: u8, cookie: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(5 + cookie.len());
    packet.extend_from_slice(MAGIC);
    packet.push(kind);
    packet.extend_from_slice(cookie);
    packet
}

pub(crate) fn decode(packet: &[u8]) -> Option<Preflight<'_>> {
    let (&kind, payload) = packet.strip_prefix(MAGIC)?.split_first()?;
    match (kind, payload.len()) {
        (HELLO, 0 | COOKIE_LEN) => Some(Preflight::Hello(payload)),
        (CHALLENGE, COOKIE_LEN) => Some(Preflight::Challenge(payload)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode_challenge, encode_hello, Preflight};

    #[test]
    fn preflight_messages_are_distinct_and_length_checked() {
        let cookie = [7_u8; 40];
        assert_eq!(decode(&encode_hello(&[])), Some(Preflight::Hello(&[])));
        assert_eq!(
            decode(&encode_hello(&cookie)),
            Some(Preflight::Hello(cookie.as_slice()))
        );
        assert_eq!(
            decode(&encode_challenge(&cookie)),
            Some(Preflight::Challenge(cookie.as_slice()))
        );
        assert_eq!(decode(b"RNKP\x01short"), None);
        assert_eq!(decode(b"not-preflight"), None);
    }
}
