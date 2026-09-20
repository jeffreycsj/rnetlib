//! Narrow parser surfaces enabled only for out-of-tree fuzz targets.

use crate::cookie::CookieGuard;
use crate::kcp_preflight;
use std::net::{Ipv4Addr, SocketAddr};

/// Exercises stateless datagram preflight and cookie validation without allocating a session.
pub fn datagram_preflight(input: &[u8]) {
    let _ = kcp_preflight::decode(input);
    let octets = [
        input.first().copied().unwrap_or(127),
        input.get(1).copied().unwrap_or(0),
        input.get(2).copied().unwrap_or(0),
        input.get(3).copied().unwrap_or(1),
    ];
    let port = input
        .get(4..6)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_be_bytes)
        .unwrap_or(1);
    let peer = SocketAddr::from((Ipv4Addr::from(octets), port));
    if let Ok(guard) = CookieGuard::new() {
        let issued = guard.issue(peer);
        debug_assert!(guard.validate(peer, &issued));
        let _ = guard.validate(peer, input);
    }
}
