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
            } else {
                socket
                    .send_to(&encode_challenge(conv, &cookie.issue(peer)), peer)
                    .await?;
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
