//! Deferred adaptive datagram sends released after security barriers complete.

use crate::adaptive_codec::{data_record, protected_record};
use crate::adaptive_peer::Peer;
use crate::adaptive_wire::DatagramWire;
use crate::auto_rekey::AutoRekey;
use crate::state::{remove_session_with_reason, session_active, Outbound, OutboundKind, Shared};
use rnet_protocol::control::ProtectedKind;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::UdpSocket;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn flush_deferred(
    shared: &Arc<Shared>,
    socket: &UdpSocket,
    wire: &mut DatagramWire,
    peers: &mut HashMap<SocketAddr, Peer>,
    deferred: &mut HashMap<SocketAddr, VecDeque<Outbound>>,
    deferred_count: &mut usize,
    scratch_peers: &mut Vec<SocketAddr>,
    automatic_rekeys: &mut HashMap<SocketAddr, AutoRekey>,
) {
    scratch_peers.clear();
    scratch_peers.extend(deferred.keys().copied());
    for &peer in scratch_peers.iter() {
        let Some(Peer::Established {
            session,
            transport,
            controller,
            last_activity,
            ..
        }) = peers.get_mut(&peer)
        else {
            continue;
        };
        if !session_active(shared, *session) || controller.is_transitioning() {
            continue;
        }
        let Some(outbound) = deferred.get_mut(&peer).and_then(VecDeque::pop_front) else {
            continue;
        };
        *deferred_count = deferred_count.saturating_sub(1);
        let Some(outbound) = outbound.resolve_latest() else {
            if deferred.get(&peer).is_some_and(VecDeque::is_empty) {
                deferred.remove(&peer);
            }
            continue;
        };
        let record = match outbound.kind {
            OutboundKind::Data => data_record(
                transport,
                controller,
                &outbound.bytes,
                shared.config.max_body_len,
            ),
            OutboundKind::GameControl => protected_record(
                transport,
                controller.epoch(),
                ProtectedKind::GameControl,
                &outbound.bytes,
                shared.config.max_body_len,
            ),
        };
        let result = match record {
            Ok(record) => {
                wire.send(socket, peer, &record, shared.config.max_datagram_size)
                    .await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => {
                shared.latencies.record(
                    crate::metrics::LatencyKind::SendQueue,
                    outbound.queued_at.elapsed(),
                );
                shared
                    .metrics
                    .bytes_sent
                    .fetch_add(outbound.bytes.len() as u64, Ordering::Relaxed);
                if controller.mode() == rnet_protocol::control::SecurityMode::Encrypted
                    || outbound.kind == OutboundKind::GameControl
                {
                    if let Some(rekey) = automatic_rekeys.get_mut(&peer) {
                        rekey.record_encrypted_bytes(outbound.bytes.len());
                    }
                }
                *last_activity = Instant::now();
            }
            Err(error) if error.code() == rnet_core::ErrorCode::WouldBlock => {
                deferred.entry(peer).or_default().push_front(outbound);
                *deferred_count += 1;
                continue;
            }
            Err(error) => {
                shared
                    .metrics
                    .protocol_errors
                    .fetch_add(1, Ordering::Relaxed);
                let session = *session;
                let reason = error.code();
                remove_session_with_reason(shared, 0, session, reason);
                peers.remove(&peer);
                wire.remove_peer(peer);
                if let Some(queued) = deferred.remove(&peer) {
                    *deferred_count = deferred_count.saturating_sub(queued.len());
                }
                continue;
            }
        }
        if deferred.get(&peer).is_some_and(VecDeque::is_empty) {
            deferred.remove(&peer);
        }
    }
}
