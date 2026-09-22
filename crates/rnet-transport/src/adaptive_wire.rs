//! UDP control retransmission and KCP record transport for adaptive sessions.

use crate::kcp::{KcpEngine, KcpTelemetryRegistry, RustKcpEngine};
use crate::metrics::LatencyKind;
use crate::state::Shared;
use bytes::BytesMut;
use rnet_core::{ErrorCode, Result, RnetError, Transport};
use rnet_protocol::control::{encode_record, Record};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const RETRY_INTERVAL: Duration = Duration::from_millis(100);
const MAX_RETRY_ATTEMPTS: u8 = 50;
const MAX_KCP_UPDATES_PER_FLUSH: usize = 256;

/// Carries complete adaptive records over raw UDP or a per-peer KCP engine.
///
/// Raw UDP keeps one stop-and-wait control record per peer. Exact duplicate protected records are
/// answered from a byte-for-byte response cache, avoiding a second Noise decrypt and replay error.
pub(crate) struct DatagramWire {
    kind: Transport,
    mtu: usize,
    max_peers: usize,
    max_session_queued_bytes: usize,
    max_runtime_queued_bytes: usize,
    kcp_queued_bytes: usize,
    started: Instant,
    engines: HashMap<SocketAddr, RustKcpEngine>,
    authorized_peers: HashMap<SocketAddr, u32>,
    update_deadlines: BinaryHeap<Reverse<(u64, SocketAddr)>>,
    scheduled_updates: HashMap<SocketAddr, u64>,
    pending: HashMap<SocketAddr, PendingRecord>,
    seen: HashMap<SocketAddr, SeenRecord>,
    active_input: Option<(SocketAddr, Vec<u8>)>,
    active_pending: Option<(SocketAddr, Vec<u8>)>,
    scratch_peers: Vec<SocketAddr>,
    output_packets: Vec<(SocketAddr, Vec<u8>)>,
    telemetry: Option<(u64, Arc<KcpTelemetryRegistry>)>,
}

struct PendingRecord {
    bytes: Vec<u8>,
    last_sent: Instant,
    attempts: u8,
}

struct SeenRecord {
    request: Vec<u8>,
    response: Option<Vec<u8>>,
}

impl DatagramWire {
    #[cfg(test)]
    pub(crate) fn new(kind: Transport, mtu: usize, max_peers: usize) -> Self {
        Self::new_with_limits(kind, mtu, max_peers, 4 * 1024 * 1024, 256 * 1024 * 1024)
    }

    pub(crate) fn new_with_limits(
        kind: Transport,
        mtu: usize,
        max_peers: usize,
        max_session_queued_bytes: usize,
        max_runtime_queued_bytes: usize,
    ) -> Self {
        Self {
            kind,
            mtu,
            max_peers,
            max_session_queued_bytes,
            max_runtime_queued_bytes,
            kcp_queued_bytes: 0,
            started: Instant::now(),
            engines: HashMap::new(),
            authorized_peers: HashMap::new(),
            update_deadlines: BinaryHeap::new(),
            scheduled_updates: HashMap::new(),
            pending: HashMap::new(),
            seen: HashMap::new(),
            active_input: None,
            active_pending: None,
            scratch_peers: Vec::new(),
            output_packets: Vec::new(),
            telemetry: None,
        }
    }

    pub(crate) fn with_telemetry(
        mut self,
        endpoint: u64,
        registry: Arc<KcpTelemetryRegistry>,
    ) -> Self {
        self.telemetry = Some((endpoint, registry));
        self
    }

    /// Releases all transport state owned by one remote address.
    pub(crate) fn remove_peer(&mut self, peer: SocketAddr) {
        if let Some(engine) = self.engines.remove(&peer) {
            self.kcp_queued_bytes = self.kcp_queued_bytes.saturating_sub(engine.queued_bytes());
        }
        if let Some((endpoint, registry)) = &self.telemetry {
            registry.remove(*endpoint, peer);
        }
        self.authorized_peers.remove(&peer);
        self.scheduled_updates.remove(&peer);
        self.pending.remove(&peer);
        self.seen.remove(&peer);
        if self
            .active_input
            .as_ref()
            .is_some_and(|value| value.0 == peer)
        {
            self.active_input = None;
        }
        if self
            .active_pending
            .as_ref()
            .is_some_and(|value| value.0 == peer)
        {
            self.active_pending = None;
        }
    }

    /// Allows one authenticated address to allocate KCP state on its next packet or send.
    pub(crate) fn authorize_peer(&mut self, peer: SocketAddr, conv: u32) -> Result<()> {
        if conv == 0 {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "KCP conversation must be nonzero",
            ));
        }
        if self.engines.contains_key(&peer) {
            return Ok(());
        }
        if let Some(current) = self.authorized_peers.get(&peer) {
            return if *current == conv {
                Ok(())
            } else {
                Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "KCP preflight changed an authorized conversation",
                ))
            };
        }
        if self
            .engines
            .len()
            .saturating_add(self.authorized_peers.len())
            >= self.max_peers
        {
            return Err(RnetError::new(
                ErrorCode::RateLimited,
                "KCP peer limit reached",
            ));
        }
        self.authorized_peers.insert(peer, conv);
        Ok(())
    }

    pub(crate) fn has_peer(&self, peer: SocketAddr) -> bool {
        self.engines.contains_key(&peer)
    }

    pub(crate) fn revoke_authorization(&mut self, peer: SocketAddr) {
        self.authorized_peers.remove(&peer);
    }

    /// Returns zero or more complete records delivered by one socket datagram.
    pub(crate) fn receive(&mut self, peer: SocketAddr, packet: &[u8]) -> Result<Vec<Vec<u8>>> {
        if self.kind != Transport::Kcp {
            return Ok(vec![packet.to_vec()]);
        }
        let authorized_conv = if self.engines.contains_key(&peer) {
            None
        } else {
            Some(self.authorized_peers.remove(&peer).ok_or_else(|| {
                RnetError::new(
                    ErrorCode::AuthRejected,
                    "KCP preflight authorization required",
                )
            })?)
        };
        let now = self.started.elapsed().as_millis() as u64;
        if self.engines.len() >= self.max_peers && !self.engines.contains_key(&peer) {
            return Err(RnetError::new(
                ErrorCode::RateLimited,
                "KCP peer limit reached",
            ));
        }
        let (records, deadline, before, after) = {
            let mut created = false;
            let engine = match self.engines.entry(peer) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let mut engine = RustKcpEngine::new_with_limits(
                        authorized_conv.expect("vacant KCP engine has preflight conversation"),
                        self.mtu,
                        self.max_session_queued_bytes,
                    )?;
                    engine.input(packet, now)?;
                    created = true;
                    if let Some((endpoint, registry)) = &self.telemetry {
                        registry.insert_engine(*endpoint, peer, &engine);
                    }
                    entry.insert(engine)
                }
            };
            let before = engine.queued_bytes();
            if !created {
                engine.input(packet, now)?;
            }
            let mut records = Vec::new();
            loop {
                let mut record = BytesMut::new();
                match engine.recv(&mut record)? {
                    Some(_) => records.push(record.to_vec()),
                    None => break,
                }
            }
            (
                records,
                engine.next_update_ms(),
                before,
                engine.queued_bytes(),
            )
        };
        self.reconcile_kcp_bytes(before, after);
        self.schedule_update(peer, deadline);
        Ok(records)
    }

    /// Sends application data once for UDP, or queues it in KCP's reliable stream.
    pub(crate) async fn send(
        &mut self,
        socket: &UdpSocket,
        peer: SocketAddr,
        record: &Record,
        max: usize,
    ) -> Result<()> {
        let bytes = encode_record(
            record,
            if self.kind == Transport::Kcp {
                u32::MAX as usize
            } else {
                max
            },
        )?;
        if self.kind == Transport::Kcp {
            self.queue_kcp(peer, &bytes, false)?;
            self.schedule_update(peer, self.started.elapsed().as_millis() as u64);
            Ok(())
        } else {
            socket.send_to(&bytes, peer).await?;
            Ok(())
        }
    }

    /// Sends a handshake/control record and retains its exact ciphertext for UDP retransmission.
    pub(crate) async fn send_reliable(
        &mut self,
        socket: &UdpSocket,
        peer: SocketAddr,
        record: &Record,
        max: usize,
    ) -> Result<()> {
        let bytes = encode_record(record, max)?;
        if self.kind == Transport::Kcp {
            self.queue_kcp(peer, &bytes, true)?;
            self.schedule_update(peer, self.started.elapsed().as_millis() as u64);
            return Ok(());
        }
        if self.pending.len() >= self.max_peers && !self.pending.contains_key(&peer) {
            return Err(RnetError::new(
                ErrorCode::RateLimited,
                "UDP control limit reached",
            ));
        }
        socket.send_to(&bytes, peer).await?;
        self.pending.insert(
            peer,
            PendingRecord {
                bytes: bytes.clone(),
                last_sent: Instant::now(),
                attempts: 1,
            },
        );
        if let Some((input_peer, request)) = &self.active_input {
            if *input_peer == peer
                && (self.seen.len() < self.max_peers || self.seen.contains_key(&peer))
            {
                self.seen.insert(
                    peer,
                    SeenRecord {
                        request: request.clone(),
                        response: Some(bytes),
                    },
                );
            }
        }
        Ok(())
    }

    /// Sends a terminal response without waiting for a response to that response.
    pub(crate) async fn send_response(
        &mut self,
        socket: &UdpSocket,
        peer: SocketAddr,
        record: &Record,
        max: usize,
    ) -> Result<()> {
        let bytes = encode_record(record, max)?;
        if self.kind == Transport::Kcp {
            self.queue_kcp(peer, &bytes, true)?;
            self.schedule_update(peer, self.started.elapsed().as_millis() as u64);
            return Ok(());
        }
        socket.send_to(&bytes, peer).await?;
        if let Some((input_peer, request)) = &self.active_input {
            if *input_peer == peer
                && (self.seen.len() < self.max_peers || self.seen.contains_key(&peer))
            {
                self.seen.insert(
                    peer,
                    SeenRecord {
                        request: request.clone(),
                        response: Some(bytes),
                    },
                );
            }
        }
        Ok(())
    }

    /// Marks an inbound protected/control record and returns a cached duplicate response.
    pub(crate) fn begin_input(
        &mut self,
        peer: SocketAddr,
        bytes: &[u8],
    ) -> (bool, Option<Vec<u8>>) {
        if self.kind == Transport::Kcp {
            return (false, None);
        }
        if let Some(seen) = self.seen.get(&peer) {
            if seen.request == bytes {
                return (true, seen.response.clone());
            }
        }
        if self.seen.len() < self.max_peers || self.seen.contains_key(&peer) {
            self.seen.insert(
                peer,
                SeenRecord {
                    request: bytes.to_vec(),
                    response: None,
                },
            );
        }
        self.active_input = Some((peer, bytes.to_vec()));
        self.active_pending = self
            .pending
            .get(&peer)
            .map(|pending| (peer, pending.bytes.clone()));
        (false, None)
    }

    /// A pending request is acknowledged only after its response was parsed successfully.
    pub(crate) fn end_input(&mut self, success: bool) {
        if success {
            if let Some((peer, old_bytes)) = &self.active_pending {
                let still_old = self
                    .pending
                    .get(peer)
                    .is_some_and(|pending| pending.bytes == *old_bytes);
                if still_old {
                    self.pending.remove(peer);
                }
            }
        }
        self.active_input = None;
        self.active_pending = None;
    }

    /// Retransmits due UDP control records without advancing Noise nonces.
    pub(crate) async fn retry(&mut self, socket: &UdpSocket) -> Vec<SocketAddr> {
        if self.kind == Transport::Kcp {
            return Vec::new();
        }
        let now = Instant::now();
        let mut expired = Vec::new();
        self.scratch_peers.clear();
        self.scratch_peers.extend(self.pending.keys().copied());
        for index in 0..self.scratch_peers.len() {
            let peer = self.scratch_peers[index];
            let Some(pending) = self.pending.get_mut(&peer) else {
                continue;
            };
            if now.duration_since(pending.last_sent) < RETRY_INTERVAL {
                continue;
            }
            if pending.attempts >= MAX_RETRY_ATTEMPTS {
                self.pending.remove(&peer);
                expired.push(peer);
                continue;
            }
            let _ = socket.send_to(&pending.bytes, peer).await;
            pending.last_sent = now;
            pending.attempts += 1;
        }
        expired
    }

    /// Flushes KCP packets produced by record sends and acknowledgement processing.
    pub(crate) async fn flush(&mut self, socket: &UdpSocket, shared: &Shared) {
        if self.kind != Transport::Kcp {
            return;
        }
        let now = self.started.elapsed().as_millis() as u64;
        let mut updated = 0;
        while updated < MAX_KCP_UPDATES_PER_FLUSH {
            let Some(Reverse((deadline, peer))) = self.update_deadlines.peek().copied() else {
                break;
            };
            if deadline > now {
                break;
            }
            self.update_deadlines.pop();
            if self.scheduled_updates.get(&peer) != Some(&deadline) {
                continue;
            }
            let Some(engine) = self.engines.get_mut(&peer) else {
                self.scheduled_updates.remove(&peer);
                continue;
            };
            self.output_packets.clear();
            if let Some(rtt) = engine.observed_rtt() {
                shared.latencies.record(LatencyKind::KcpRtt, rtt);
            }
            engine.update(now, &mut |packet| {
                self.output_packets.push((peer, packet.to_vec()))
            });
            let next = engine.next_update_ms();
            self.scheduled_updates.insert(peer, next);
            self.update_deadlines.push(Reverse((next, peer)));
            updated += 1;
            let packets = std::mem::take(&mut self.output_packets);
            for (packet_peer, packet) in &packets {
                if socket.send_to(packet, packet_peer).await.is_err() {
                    shared
                        .metrics
                        .protocol_errors
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            self.output_packets = packets;
            self.output_packets.clear();
        }
    }

    fn engine(&mut self, peer: SocketAddr) -> Result<&mut RustKcpEngine> {
        let authorized_conv = if self.engines.contains_key(&peer) {
            None
        } else {
            Some(self.authorized_peers.remove(&peer).ok_or_else(|| {
                RnetError::new(
                    ErrorCode::AuthRejected,
                    "KCP preflight authorization required",
                )
            })?)
        };
        if self.engines.len() >= self.max_peers && !self.engines.contains_key(&peer) {
            return Err(RnetError::new(
                ErrorCode::RateLimited,
                "KCP peer limit reached",
            ));
        }
        match self.engines.entry(peer) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let engine = RustKcpEngine::new_with_limits(
                    authorized_conv.expect("vacant KCP engine has preflight conversation"),
                    self.mtu,
                    self.max_session_queued_bytes,
                )?;
                if let Some((endpoint, registry)) = &self.telemetry {
                    registry.insert_engine(*endpoint, peer, &engine);
                }
                Ok(entry.insert(engine))
            }
        }
    }

    fn queue_kcp(&mut self, peer: SocketAddr, bytes: &[u8], control: bool) -> Result<()> {
        let current_total = self.kcp_queued_bytes;
        let max_runtime_queued_bytes = self.max_runtime_queued_bytes;
        let data_runtime_limit =
            max_runtime_queued_bytes.saturating_sub((max_runtime_queued_bytes / 16).min(64 * 1024));
        let max_session_queued_bytes = self.max_session_queued_bytes;
        let data_session_limit =
            max_session_queued_bytes.saturating_sub((max_session_queued_bytes / 16).min(4 * 1024));
        let (before, added, after) = {
            let engine = self.engine(peer)?;
            let before = engine.queued_bytes();
            let added = engine.queued_bytes_for(bytes.len());
            let runtime_limit = if control {
                max_runtime_queued_bytes
            } else {
                data_runtime_limit
            };
            let session_limit = if control {
                max_session_queued_bytes
            } else {
                data_session_limit
            };
            if current_total.saturating_add(added) > runtime_limit
                || before.saturating_add(added) > session_limit
            {
                return Err(RnetError::new(
                    ErrorCode::WouldBlock,
                    "KCP unacknowledged-byte budget is full",
                ));
            }
            engine.send(bytes)?;
            (before, added, engine.queued_bytes())
        };
        debug_assert!(after.saturating_sub(before) <= added);
        self.reconcile_kcp_bytes(before, after);
        Ok(())
    }

    fn reconcile_kcp_bytes(&mut self, before: usize, after: usize) {
        self.kcp_queued_bytes = self
            .kcp_queued_bytes
            .saturating_sub(before)
            .saturating_add(after);
    }

    fn schedule_update(&mut self, peer: SocketAddr, deadline: u64) {
        self.scheduled_updates.insert(peer, deadline);
        self.update_deadlines.push(Reverse((deadline, peer)));
    }
}

impl Drop for DatagramWire {
    fn drop(&mut self) {
        if let Some((endpoint, registry)) = &self.telemetry {
            for peer in self.engines.keys() {
                registry.remove(*endpoint, *peer);
            }
        }
    }
}

#[cfg(test)]
#[path = "adaptive_wire_tests.rs"]
mod tests;
