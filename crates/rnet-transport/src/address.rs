//! Bounded DNS resolution and deterministic connection-candidate ordering.

use rnet_core::{ErrorCode, Result, RnetError};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

const MAX_CONCURRENT_RESOLVERS: usize = 64;
const MAX_ADDRESS_CANDIDATES: usize = 32;
static ACTIVE_RESOLVERS: AtomicUsize = AtomicUsize::new(0);

struct ResolverPermit;

impl ResolverPermit {
    // `try_update` is the future spelling, but it is unavailable on the declared Rust 1.85 MSRV.
    #[allow(deprecated)]
    fn acquire() -> Result<Self> {
        ACTIVE_RESOLVERS
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |active| {
                (active < MAX_CONCURRENT_RESOLVERS).then_some(active + 1)
            })
            .map_err(|_| {
                RnetError::new(
                    ErrorCode::WouldBlock,
                    "DNS resolver concurrency limit reached",
                )
            })?;
        Ok(Self)
    }
}

impl Drop for ResolverPermit {
    fn drop(&mut self) {
        ACTIVE_RESOLVERS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn resolve_host(host: &str, port: u16, timeout: Duration) -> Result<Vec<SocketAddr>> {
    let host = host.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    let permit = ResolverPermit::acquire()?;
    std::thread::Builder::new()
        .name("rnet-dns".to_owned())
        .spawn(move || {
            let _permit = permit;
            let result = (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| normalize_candidates(addresses.collect()));
            let _ = sender.send(result);
        })
        .map_err(RnetError::from)?;
    let addresses = receiver
        .recv_timeout(timeout)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => {
                RnetError::new(ErrorCode::Timeout, "DNS resolution timed out")
            }
            mpsc::RecvTimeoutError::Disconnected => {
                RnetError::new(ErrorCode::IoError, "DNS resolver stopped unexpectedly")
            }
        })?
        .map_err(|error| RnetError::new(ErrorCode::IoError, error.to_string()))?;
    if addresses.is_empty() {
        return Err(RnetError::new(
            ErrorCode::IoError,
            "host name did not resolve to an IP address",
        ));
    }
    Ok(addresses)
}

fn normalize_candidates(mut addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    addresses.sort_by_key(|address| u8::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(MAX_ADDRESS_CANDIDATES);
    addresses
}

#[cfg(test)]
mod tests {
    use super::{normalize_candidates, MAX_ADDRESS_CANDIDATES};

    #[test]
    fn candidates_are_deduplicated_with_ipv4_first() {
        let v4 = "127.0.0.1:80".parse().unwrap();
        let v6 = "[::1]:80".parse().unwrap();
        assert_eq!(normalize_candidates(vec![v6, v4, v4]), vec![v4, v6]);
    }

    #[test]
    fn candidate_count_is_bounded_before_connection_attempts() {
        let addresses = (1..=40)
            .map(|port| format!("127.0.0.1:{port}").parse().unwrap())
            .collect();

        assert_eq!(
            normalize_candidates(addresses).len(),
            MAX_ADDRESS_CANDIDATES
        );
    }
}
