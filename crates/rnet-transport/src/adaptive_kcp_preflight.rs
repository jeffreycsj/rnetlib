//! Stateless KCP address validation before any per-peer KCP engine is allocated.

use crate::adaptive_wire::DatagramWire;
use crate::admission::{AdmissionController, AdmissionPermit};
use crate::config::EndpointSecurity;
use crate::cookie::{CookieGuard, COOKIE_LEN};
use crate::kcp_preflight::{decode, encode_challenge, encode_hello, Preflight};
use crate::metrics::AdmissionRejectReason;
use crate::state::Shared;
use rnet_core::{Result, Transport};
use rnet_protocol::control::{Record, RecordKind};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use tokio::net::UdpSocket;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_preflight(
    shared: &Shared,
    socket: &UdpSocket,
    wire: &mut DatagramWire,
    transport: Transport,
    security: &EndpointSecurity,
    remote: Option<SocketAddr>,
    cookie: &CookieGuard,
    per_ip: &AdmissionController,
    permits: &mut HashMap<SocketAddr, AdmissionPermit>,
    authorized: &mut HashMap<SocketAddr, Instant>,
    peer: SocketAddr,
    packet: &[u8],
) -> Result<bool> {
    if transport != Transport::Kcp || wire.has_peer(peer) {
        return Ok(false);
    }
    match decode(packet) {
        Some(Preflight::Hello {
            conv,
            cookie: client_cookie,
        }) if matches!(security, EndpointSecurity::AdaptiveServer { .. }) => {
            if client_cookie.len() == COOKIE_LEN && cookie.validate(peer, client_cookie) {
                if let std::collections::hash_map::Entry::Vacant(entry) = permits.entry(peer) {
                    let permit = match per_ip.try_acquire(peer.ip()) {
                        Ok(permit) => permit,
                        Err(reason) => {
                            shared
                                .metrics
                                .record_admission_rejected(reason.metric_reason());
                            return Ok(true);
                        }
                    };
                    entry.insert(permit);
                }
                if wire.authorize_peer(peer, conv).is_ok() {
                    authorized.insert(peer, Instant::now() + shared.config.handshake_timeout);
                } else {
                    permits.remove(&peer);
                    shared
                        .metrics
                        .record_admission_rejected(AdmissionRejectReason::KcpPeerLimit);
                }
            } else if socket
                .send_to(&encode_challenge(conv, &cookie.issue(peer)), peer)
                .await
                .is_err()
            {
                // An unverified source owns no session. A failed challenge must not
                // terminate the shared listener or produce attacker-controlled log volume.
                shared
                    .metrics
                    .record_admission_rejected(AdmissionRejectReason::Unauthenticated);
            }
            Ok(true)
        }
        Some(Preflight::Challenge {
            conv,
            cookie: server_cookie,
        }) if matches!(security, EndpointSecurity::AdaptiveClient { .. })
            && remote == Some(peer) =>
        {
            socket
                .send_to(&encode_hello(conv, server_cookie), peer)
                .await?;
            wire.authorize_peer(peer, conv)?;
            wire.send_reliable(
                socket,
                peer,
                &Record::new(RecordKind::ClientHello, 0, server_cookie),
                128,
            )
            .await?;
            Ok(true)
        }
        Some(_) => Ok(true),
        None if authorized.contains_key(&peer) => Ok(false),
        None => {
            shared
                .metrics
                .record_admission_rejected(AdmissionRejectReason::Unauthenticated);
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        handle_preflight, AdmissionController, CookieGuard, DatagramWire, EndpointSecurity,
    };
    use crate::kcp_preflight::{encode_challenge, encode_hello};
    use crate::{NetworkRuntime, RuntimeConfig};
    use rnet_core::{ErrorCode, Transport};
    use rnet_protocol::control::SecurityMode;
    use rnet_security::Keypair;
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::net::UdpSocket;

    #[test]
    fn preflight_send_failure_is_isolated_on_server_but_reported_on_client() {
        let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
        runtime.poll_events(8, Duration::ZERO);
        runtime.runtime.block_on(async {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            // Exercise a real OS send failure without raw sockets or external traffic.
            let peer = "[::1]:9".parse().unwrap();
            let cookie = CookieGuard::new().unwrap();
            let admission = AdmissionController::new(8, 8, 8, 8, 64);
            let mut wire = DatagramWire::new(Transport::Kcp, 1200, 8);
            let (mut permits, mut authorized) = (HashMap::new(), HashMap::new());
            let server = EndpointSecurity::AdaptiveServer {
                local_key: Keypair::generate().unwrap(),
                initial_mode: SecurityMode::Encrypted,
            };
            let client = EndpointSecurity::AdaptiveClient {
                join_payload: Vec::new(),
            };
            for (security, packet) in [
                (&server, encode_hello(37, &[])),
                (&client, encode_challenge(37, &cookie.issue(peer))),
            ] {
                let result = handle_preflight(
                    &runtime.shared,
                    &socket,
                    &mut wire,
                    Transport::Kcp,
                    security,
                    Some(peer),
                    &cookie,
                    &admission,
                    &mut permits,
                    &mut authorized,
                    peer,
                    &packet,
                )
                .await;
                if matches!(security, EndpointSecurity::AdaptiveServer { .. }) {
                    assert!(
                        result.unwrap(),
                        "failed challenge must not terminate the listener"
                    );
                } else {
                    assert_eq!(result.unwrap_err().code(), ErrorCode::IoError);
                }
                assert!(permits.is_empty() && authorized.is_empty());
                assert!(!wire.has_peer(peer));
            }
            assert_eq!(runtime.metrics_snapshot().admission_rejected, 1);
            assert!(runtime.poll_events(8, Duration::ZERO).is_empty());
        });
        runtime.stop(Duration::ZERO).unwrap();
    }
}
