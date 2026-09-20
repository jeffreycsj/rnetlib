use ring::hmac;
use ring::rand::SecureRandom;
use ring::rand::SystemRandom;
use rnet_core::ErrorCode;
use rnet_core::Result;
use rnet_core::RnetError;
use std::net::SocketAddr;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub(crate) const COOKIE_LEN: usize = 40;

pub(crate) struct CookieGuard {
    key: hmac::Key,
}

impl CookieGuard {
    pub(crate) fn new() -> Result<Self> {
        let mut secret = [0_u8; 32];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| RnetError::new(ErrorCode::CryptoError, "cookie RNG failed"))?;
        Ok(Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, &secret),
        })
    }

    pub(crate) fn issue(&self, peer: SocketAddr) -> [u8; COOKIE_LEN] {
        let bucket = cookie_time_bucket();
        let tag = self.sign(peer, bucket);
        let mut cookie = [0; COOKIE_LEN];
        cookie[..8].copy_from_slice(&bucket.to_be_bytes());
        cookie[8..].copy_from_slice(tag.as_ref());
        cookie
    }

    pub(crate) fn validate(&self, peer: SocketAddr, cookie: &[u8]) -> bool {
        if cookie.len() != COOKIE_LEN {
            return false;
        }
        let bucket = u64::from_be_bytes(cookie[..8].try_into().expect("cookie bucket"));
        let current = cookie_time_bucket();
        if bucket != current && bucket.saturating_add(1) != current {
            return false;
        }
        hmac::verify(&self.key, &cookie_input(peer, bucket), &cookie[8..]).is_ok()
    }

    fn sign(&self, peer: SocketAddr, bucket: u64) -> hmac::Tag {
        hmac::sign(&self.key, &cookie_input(peer, bucket))
    }
}

fn cookie_time_bucket() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 60
}

fn cookie_input(peer: SocketAddr, bucket: u64) -> Vec<u8> {
    let mut input = Vec::with_capacity(32);
    match peer.ip() {
        std::net::IpAddr::V4(ip) => input.extend_from_slice(&ip.octets()),
        std::net::IpAddr::V6(ip) => input.extend_from_slice(&ip.octets()),
    }
    input.extend_from_slice(&peer.port().to_be_bytes());
    input.extend_from_slice(&bucket.to_be_bytes());
    input
}
