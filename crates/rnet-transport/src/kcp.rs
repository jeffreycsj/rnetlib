use bytes::BytesMut;
use rnet_core::ErrorCode;
use rnet_core::Result;
use rnet_core::RnetError;
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

pub trait KcpEngine: Send {
    fn input(&mut self, packet: &[u8], now_ms: u64) -> Result<()>;
    fn send(&mut self, payload: &[u8]) -> Result<()>;
    fn recv(&mut self, out: &mut BytesMut) -> Result<Option<usize>>;
    fn update(&mut self, now_ms: u64, output: &mut dyn FnMut(&[u8]));
    fn next_update_ms(&self) -> u64;
    fn observed_rtt(&self) -> Option<Duration>;
}

#[derive(Clone, Default)]
struct KcpPacketOutput(Arc<Mutex<VecDeque<Vec<u8>>>>);

impl Write for KcpPacketOutput {
    fn write(&mut self, packet: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("KCP output queue poisoned")
            .push_back(packet.to_vec());
        Ok(packet.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct RustKcpEngine {
    inner: ::kcp::Kcp<KcpPacketOutput>,
    output: KcpPacketOutput,
    now_ms: u64,
    observed_rtt: Option<Duration>,
    max_queued_bytes: usize,
    failed: bool,
    transmitted_sequences: HashSet<u32>,
    telemetry: Arc<Mutex<KcpTelemetry>>,
}

/// Counts KCP PUSH segment retransmissions, not IP-layer packet loss.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KcpRetransmissionSnapshot {
    pub segments_sent: u64,
    pub retransmitted: u64,
    pub recent_segments: u16,
    pub recent_retransmitted: u16,
    pub recent_retransmission_per_mille: u16,
}

#[derive(Default)]
struct KcpTelemetry {
    segments_sent: u64,
    retransmitted: u64,
    recent: VecDeque<bool>,
    recent_retransmitted: u16,
}

impl KcpTelemetry {
    fn record(&mut self, retransmission: bool) {
        const WINDOW: usize = 128;
        self.segments_sent = self.segments_sent.saturating_add(1);
        self.retransmitted = self.retransmitted.saturating_add(u64::from(retransmission));
        if self.recent.len() == WINDOW && self.recent.pop_front() == Some(true) {
            self.recent_retransmitted -= 1;
        }
        self.recent.push_back(retransmission);
        self.recent_retransmitted += u16::from(retransmission);
    }

    fn snapshot(&self) -> KcpRetransmissionSnapshot {
        let recent_segments = self.recent.len() as u16;
        KcpRetransmissionSnapshot {
            segments_sent: self.segments_sent,
            retransmitted: self.retransmitted,
            recent_segments,
            recent_retransmitted: self.recent_retransmitted,
            recent_retransmission_per_mille: u32::from(self.recent_retransmitted)
                .saturating_mul(1000)
                .checked_div(u32::from(recent_segments))
                .unwrap_or(0) as u16,
        }
    }
}

/// Bounded by active KCP peers; removed with the peer and when its endpoint task exits.
type KcpPeerTelemetry = Arc<Mutex<KcpTelemetry>>;

#[derive(Default)]
pub(crate) struct KcpTelemetryRegistry {
    peers: Mutex<HashMap<(u64, std::net::SocketAddr), KcpPeerTelemetry>>,
}

impl KcpTelemetryRegistry {
    pub(crate) fn insert_engine(
        &self,
        endpoint: u64,
        peer: std::net::SocketAddr,
        engine: &RustKcpEngine,
    ) {
        self.peers
            .lock()
            .expect("KCP telemetry table poisoned")
            .insert((endpoint, peer), engine.telemetry());
    }

    pub(crate) fn remove(&self, endpoint: u64, peer: std::net::SocketAddr) {
        self.peers
            .lock()
            .expect("KCP telemetry table poisoned")
            .remove(&(endpoint, peer));
    }

    pub(crate) fn snapshot(
        &self,
        endpoint: u64,
        peer: std::net::SocketAddr,
    ) -> Option<KcpRetransmissionSnapshot> {
        let telemetry = self
            .peers
            .lock()
            .expect("KCP telemetry table poisoned")
            .get(&(endpoint, peer))
            .cloned()?;
        let snapshot = telemetry.lock().expect("KCP telemetry poisoned").snapshot();
        Some(snapshot)
    }
}

impl RustKcpEngine {
    pub fn new(conv: u32) -> Result<Self> {
        Self::new_with_mtu(conv, 1200)
    }

    /// Creates an engine whose emitted packets never exceed the endpoint datagram MTU.
    pub fn new_with_mtu(conv: u32, mtu: usize) -> Result<Self> {
        Self::new_with_limits(conv, mtu, 4 * 1024 * 1024)
    }

    /// Creates an engine with a hard upper bound for unacknowledged payload storage.
    pub fn new_with_limits(conv: u32, mtu: usize, max_queued_bytes: usize) -> Result<Self> {
        if max_queued_bytes == 0 {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "KCP queued-byte limit must be positive",
            ));
        }
        let output = KcpPacketOutput::default();
        let mut inner = ::kcp::Kcp::new(conv, output.clone());
        inner.set_nodelay(true, 10, 2, true);
        inner.set_wndsize(128, 128);
        inner.set_mtu(mtu).map_err(map_kcp_error)?;
        Ok(Self {
            inner,
            output,
            now_ms: 0,
            observed_rtt: None,
            max_queued_bytes,
            failed: false,
            transmitted_sequences: HashSet::new(),
            telemetry: Arc::new(Mutex::new(KcpTelemetry::default())),
        })
    }

    pub(crate) fn queued_bytes(&self) -> usize {
        self.inner.wait_snd().saturating_mul(self.inner.mss())
    }

    pub(crate) fn queued_bytes_for(&self, payload_len: usize) -> usize {
        payload_len
            .div_ceil(self.inner.mss())
            .saturating_mul(self.inner.mss())
    }

    fn telemetry(&self) -> Arc<Mutex<KcpTelemetry>> {
        Arc::clone(&self.telemetry)
    }

    pub fn retransmission_snapshot(&self) -> KcpRetransmissionSnapshot {
        self.telemetry
            .lock()
            .expect("KCP telemetry poisoned")
            .snapshot()
    }
}

impl KcpEngine for RustKcpEngine {
    fn input(&mut self, packet: &[u8], now_ms: u64) -> Result<()> {
        if self.failed {
            return Err(failed_engine());
        }
        validate_kcp_packet(packet, now_ms as u32, self.inner.mss())?;
        self.now_ms = now_ms;
        match std::panic::catch_unwind(AssertUnwindSafe(|| self.inner.input(packet))) {
            Ok(result) => {
                result.map_err(map_kcp_error)?;
            }
            Err(_) => {
                self.failed = true;
                self.output
                    .0
                    .lock()
                    .expect("KCP output queue poisoned")
                    .clear();
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "KCP rejected malformed input with an internal panic",
                ));
            }
        }
        self.observed_rtt = kcp_ack_rtt(packet, self.now_ms as u32)
            .map(|rtt_ms| Duration::from_millis(u64::from(rtt_ms)));
        // Only a successfully accepted ACK/UNA may retire an outstanding sequence. The set is
        // bounded by KCP's send window rather than the lifetime of a long-running connection.
        retire_acked_sequences(packet, &mut self.transmitted_sequences);
        Ok(())
    }

    fn send(&mut self, payload: &[u8]) -> Result<()> {
        if self.failed {
            return Err(failed_engine());
        }
        if self
            .queued_bytes()
            .saturating_add(self.queued_bytes_for(payload.len()))
            > self.max_queued_bytes
        {
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "KCP unacknowledged-byte budget is full",
            ));
        }
        self.inner.send(payload).map_err(map_kcp_error)?;
        Ok(())
    }

    fn recv(&mut self, out: &mut BytesMut) -> Result<Option<usize>> {
        if self.failed {
            return Err(failed_engine());
        }
        let length = match std::panic::catch_unwind(AssertUnwindSafe(|| self.inner.peeksize())) {
            Ok(Ok(length)) => length,
            Ok(Err(::kcp::Error::RecvQueueEmpty | ::kcp::Error::ExpectingFragment)) => {
                return Ok(None)
            }
            Ok(Err(error)) => return Err(map_kcp_error(error)),
            Err(_) => {
                self.failed = true;
                return Err(failed_engine());
            }
        };
        out.clear();
        out.resize(length, 0);
        let received =
            match std::panic::catch_unwind(AssertUnwindSafe(|| self.inner.recv(out.as_mut()))) {
                Ok(result) => result.map_err(map_kcp_error)?,
                Err(_) => {
                    self.failed = true;
                    out.clear();
                    return Err(failed_engine());
                }
            };
        out.truncate(received);
        Ok(Some(received))
    }

    fn update(&mut self, now_ms: u64, output: &mut dyn FnMut(&[u8])) {
        if self.failed {
            return;
        }
        self.now_ms = now_ms;
        if self.inner.update(self.now_ms as u32).is_err() {
            return;
        }
        let mut packets = self.output.0.lock().expect("KCP output queue poisoned");
        while let Some(packet) = packets.pop_front() {
            observe_push_segments(&packet, &mut self.transmitted_sequences, &self.telemetry);
            output(&packet);
        }
    }

    fn next_update_ms(&self) -> u64 {
        self.now_ms
            .saturating_add(u64::from(self.inner.check(self.now_ms as u32)))
    }

    fn observed_rtt(&self) -> Option<Duration> {
        self.observed_rtt
    }
}

fn observe_push_segments(
    mut packet: &[u8],
    transmitted: &mut HashSet<u32>,
    telemetry: &Mutex<KcpTelemetry>,
) {
    while packet.len() >= 24 {
        let length =
            u32::from_le_bytes(packet[20..24].try_into().expect("KCP segment length")) as usize;
        let Some(segment_length) = 24_usize.checked_add(length) else {
            break;
        };
        if segment_length > packet.len() {
            break;
        }
        if packet[4] == 81 {
            let sequence = u32::from_le_bytes(packet[12..16].try_into().expect("KCP sequence"));
            telemetry
                .lock()
                .expect("KCP telemetry poisoned")
                .record(!transmitted.insert(sequence));
        }
        packet = &packet[segment_length..];
    }
}

fn retire_acked_sequences(mut packet: &[u8], transmitted: &mut HashSet<u32>) {
    while packet.len() >= 24 {
        let length =
            u32::from_le_bytes(packet[20..24].try_into().expect("validated KCP length")) as usize;
        let segment_length = 24 + length; // Packet validation already checked this addition.
        let una = u32::from_le_bytes(packet[16..20].try_into().expect("KCP UNA"));
        transmitted.retain(|sequence| *sequence >= una);
        if packet[4] == 82 {
            let sequence = u32::from_le_bytes(packet[12..16].try_into().expect("KCP ACK sequence"));
            transmitted.remove(&sequence);
        }
        packet = &packet[segment_length..];
    }
}

/// Rejects wire values that the pinned KCP dependency assumes its peer generated correctly.
///
/// The upstream parser performs some timestamp and fragment arithmetic before returning an error;
/// malformed values can therefore overflow in checked builds. Keeping this validation at the
/// untrusted-input boundary also prevents allocations for segments larger than the negotiated MTU.
fn validate_kcp_packet(mut packet: &[u8], now_ms: u32, maximum_payload: usize) -> Result<()> {
    const OVERHEAD: usize = 24;
    const PUSH: u8 = 81;
    const ACK: u8 = 82;
    const WINDOW_PROBE: u8 = 83;
    const WINDOW_SIZE: u8 = 84;
    const MAX_ACK_RTT_MS: u32 = 60_000;
    // Every engine created by this adapter uses a 128-segment receive window. Accepting a peer's
    // larger advertisement cannot improve throughput, but it can drive the dependency through
    // arithmetic states that a conforming peer can never produce.
    const RECEIVE_WINDOW: u16 = 128;
    // kcp 0.6.0 implements serial-number comparison with signed subtraction instead of an
    // explicit wrapping subtraction. Keep attacker-controlled values away from the sign boundary.
    // This is not a substitute for rotating extremely long-lived KCP conversations before their
    // own sequence counter reaches the boundary; that remains a production qualification limit.
    const MAX_SAFE_SEQUENCE: u32 = i32::MAX as u32 - RECEIVE_WINDOW as u32;

    if packet.len() < OVERHEAD {
        return Err(RnetError::new(
            ErrorCode::ProtocolError,
            "KCP packet is shorter than its header",
        ));
    }
    while !packet.is_empty() {
        if packet.len() < OVERHEAD {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP packet has a truncated trailing segment",
            ));
        }
        let command = packet[4];
        if !matches!(command, PUSH | ACK | WINDOW_PROBE | WINDOW_SIZE) {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP packet has an unsupported command",
            ));
        }
        let advertised_window =
            u16::from_le_bytes(packet[6..8].try_into().expect("fixed KCP window field"));
        if advertised_window > RECEIVE_WINDOW {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP peer advertised a window larger than the negotiated limit",
            ));
        }
        let sequence =
            u32::from_le_bytes(packet[12..16].try_into().expect("fixed KCP sequence field"));
        let unacknowledged = u32::from_le_bytes(
            packet[16..20]
                .try_into()
                .expect("fixed KCP unacknowledged field"),
        );
        if sequence > MAX_SAFE_SEQUENCE || unacknowledged > MAX_SAFE_SEQUENCE {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP sequence exceeds the dependency's safe serial-number range",
            ));
        }
        let payload_len = u32::from_le_bytes(
            packet[20..24]
                .try_into()
                .expect("fixed KCP payload-length field"),
        ) as usize;
        if payload_len > maximum_payload {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP segment payload exceeds the negotiated MTU",
            ));
        }
        let segment_len = OVERHEAD.checked_add(payload_len).ok_or_else(|| {
            RnetError::new(ErrorCode::ProtocolError, "KCP segment length overflow")
        })?;
        if packet.len() < segment_len {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP segment payload is truncated",
            ));
        }
        if command != PUSH && payload_len != 0 {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP control segment unexpectedly contains payload",
            ));
        }
        let fragment = packet[5];
        if (command == PUSH && fragment >= 128) || (command != PUSH && fragment != 0) {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "KCP segment has an invalid fragment index",
            ));
        }
        if command == ACK {
            let timestamp =
                u32::from_le_bytes(packet[8..12].try_into().expect("fixed KCP timestamp field"));
            let signed_difference = i64::from(now_ms as i32) - i64::from(timestamp as i32);
            if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&signed_difference) {
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "KCP ACK timestamp would overflow dependency time arithmetic",
                ));
            }
            let rtt = signed_difference as i32;
            if rtt >= 0 && rtt as u32 > MAX_ACK_RTT_MS {
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "KCP ACK timestamp exceeds the safe RTT window",
                ));
            }
        }
        packet = &packet[segment_len..];
    }
    Ok(())
}

fn failed_engine() -> RnetError {
    RnetError::new(
        ErrorCode::ProtocolError,
        "KCP engine was isolated after malformed input",
    )
}

fn kcp_ack_rtt(mut packet: &[u8], now_ms: u32) -> Option<u32> {
    const OVERHEAD: usize = 24;
    const ACK: u8 = 82;
    let mut observed = None;
    while packet.len() >= OVERHEAD {
        let payload_len = u32::from_le_bytes(packet[20..24].try_into().ok()?) as usize;
        let segment_len = OVERHEAD.checked_add(payload_len)?;
        if packet.len() < segment_len {
            return None;
        }
        if packet[4] == ACK {
            let timestamp = u32::from_le_bytes(packet[8..12].try_into().ok()?);
            observed = Some(now_ms.wrapping_sub(timestamp));
        }
        packet = &packet[segment_len..];
    }
    observed
}

fn map_kcp_error(error: ::kcp::Error) -> RnetError {
    let code = match error {
        ::kcp::Error::RecvQueueEmpty | ::kcp::Error::ExpectingFragment => ErrorCode::WouldBlock,
        ::kcp::Error::UserBufTooBig | ::kcp::Error::UserBufTooSmall => ErrorCode::MessageTooLarge,
        ::kcp::Error::IoError(_) => ErrorCode::IoError,
        _ => ErrorCode::ProtocolError,
    };
    RnetError::new(code, error.to_string())
}

#[cfg(test)]
mod telemetry_tests {
    use super::{KcpEngine, RustKcpEngine};

    #[test]
    fn retransmission_counters_track_push_segments_without_ack() {
        let mut engine = RustKcpEngine::new(11).expect("KCP engine");
        engine.send(b"hello").expect("queue data");
        let mut output = Vec::new();
        engine.update(0, &mut |packet| output.push(packet.to_vec()));
        assert_eq!(engine.retransmission_snapshot().segments_sent, 1);
        assert_eq!(engine.retransmission_snapshot().retransmitted, 0);
        engine.update(1000, &mut |packet| output.push(packet.to_vec()));
        let stats = engine.retransmission_snapshot();
        assert!(
            stats.segments_sent >= 2,
            "KCP should retry unacknowledged data"
        );
        assert!(stats.retransmitted >= 1);
        assert_eq!(
            stats.recent_retransmission_per_mille,
            (u32::from(stats.recent_retransmitted) * 1000 / u32::from(stats.recent_segments))
                as u16
        );
    }
}
