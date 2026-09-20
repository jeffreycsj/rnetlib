//! Authenticated-control heartbeat format and monotonic probe state.
//!
//! This module does not send packets. Runtime wiring must place its envelopes inside the
//! Noise-protected control path, including when business data is configured as plaintext.

use crate::envelope::{encode_control, ControlKind};
use crate::event::{QualityBasis, QualityGrade};
use crate::quality::QualityChangeGate;
use bytes::Bytes;
use rnet_core::{ErrorCode, Result, RnetError};
use std::time::{Duration, Instant};

const HEARTBEAT_LEN: usize = 9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeartbeatKind {
    Probe,
    Ack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HeartbeatPacket {
    pub kind: HeartbeatKind,
    pub challenge: u64,
}

/// Encodes an internal game control; the transport must additionally authenticate this control.
pub(crate) fn encode_heartbeat(packet: HeartbeatPacket, maximum_len: usize) -> Result<Bytes> {
    let mut payload = [0; HEARTBEAT_LEN];
    payload[0] = match packet.kind {
        HeartbeatKind::Probe => 1,
        HeartbeatKind::Ack => 2,
    };
    payload[1..].copy_from_slice(&packet.challenge.to_be_bytes());
    encode_control(ControlKind::Heartbeat, &payload, maximum_len)
}

/// Parses only a heartbeat control payload, rejecting extensions until explicitly negotiated.
pub(crate) fn decode_heartbeat(payload: &[u8]) -> Result<HeartbeatPacket> {
    if payload.len() != HEARTBEAT_LEN {
        return Err(protocol_error("invalid heartbeat length"));
    }
    let kind = match payload[0] {
        1 => HeartbeatKind::Probe,
        2 => HeartbeatKind::Ack,
        _ => return Err(protocol_error("invalid heartbeat operation")),
    };
    Ok(HeartbeatPacket {
        kind,
        challenge: u64::from_be_bytes(payload[1..].try_into().expect("validated length")),
    })
}

/// A probe sample derived only from a matched challenge and local monotonic time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QualitySample {
    pub last_rtt: Duration,
    pub smoothed_rtt: Duration,
    pub jitter: Duration,
    pub samples: u64,
}

/// Tracks exactly one outstanding challenge so stale or duplicate acknowledgements cannot
/// refresh liveness or bias latency measurements.
#[derive(Debug)]
pub(crate) struct HeartbeatTracker {
    interval: Duration,
    timeout: Duration,
    last_ack: Instant,
    last_inbound_probe: Option<Instant>,
    pending: Option<(u64, Instant)>,
    sample: Option<QualitySample>,
    quality_change_gate: QualityChangeGate,
}

impl HeartbeatTracker {
    pub(crate) fn new(interval: Duration, timeout: Duration, now: Instant) -> Result<Self> {
        if interval.is_zero() || timeout.is_zero() || timeout < interval {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "heartbeat timeout must be at least the nonzero interval",
            ));
        }
        Ok(Self {
            interval,
            timeout,
            last_ack: now,
            last_inbound_probe: None,
            pending: None,
            sample: None,
            quality_change_gate: QualityChangeGate::default(),
        })
    }

    pub(crate) fn should_probe(&self, now: Instant) -> bool {
        self.pending.is_none() && now.saturating_duration_since(self.last_ack) >= self.interval
    }

    /// Limits authenticated reply work from a peer without trusting its claimed time or nonce.
    pub(crate) fn admit_probe(&mut self, received_at: Instant) -> bool {
        let spacing = (self.interval / 4).max(Duration::from_millis(1));
        if self.last_inbound_probe.is_some_and(|last| {
            received_at
                .checked_duration_since(last)
                .is_none_or(|age| age < spacing)
        }) {
            return false;
        }
        self.last_inbound_probe = Some(received_at);
        true
    }

    pub(crate) fn mark_sent(&mut self, challenge: u64, now: Instant) -> Result<()> {
        if !self.should_probe(now) {
            return Err(RnetError::new(
                ErrorCode::InvalidState,
                "heartbeat probe is not due or another probe is pending",
            ));
        }
        self.pending = Some((challenge, now));
        Ok(())
    }

    pub(crate) fn accept_ack(
        &mut self,
        challenge: u64,
        received_at: Instant,
        processed_at: Instant,
    ) -> Option<QualitySample> {
        let (expected, sent_at) = self.pending?;
        let rtt = received_at.checked_duration_since(sent_at)?;
        processed_at.checked_duration_since(received_at)?;
        if expected != challenge || rtt >= self.timeout {
            return None;
        }
        let sample = match self.sample {
            None => QualitySample {
                last_rtt: rtt,
                smoothed_rtt: rtt,
                jitter: Duration::ZERO,
                samples: 1,
            },
            Some(previous) => {
                let delta = rtt.abs_diff(previous.last_rtt);
                QualitySample {
                    last_rtt: rtt,
                    smoothed_rtt: previous
                        .smoothed_rtt
                        .saturating_sub(previous.smoothed_rtt / 8)
                        .saturating_add(rtt / 8),
                    jitter: previous
                        .jitter
                        .saturating_sub(previous.jitter / 16)
                        .saturating_add(delta / 16),
                    samples: previous.samples.saturating_add(1),
                }
            }
        };
        self.pending = None;
        // The event may sit in a bounded queue while the game loop is busy. Its enqueue time
        // measures RTT, but scheduling from that stale time would immediately false-timeout.
        self.last_ack = processed_at;
        self.sample = Some(sample);
        Some(sample)
    }

    pub(crate) fn timed_out(&self, now: Instant) -> bool {
        match self.pending {
            Some((_, sent_at)) => now.saturating_duration_since(sent_at) >= self.timeout,
            None => {
                // A saturated local send queue must not keep a dead session alive forever just
                // because no probe could be enqueued.
                now.saturating_duration_since(self.last_ack)
                    >= self.interval.saturating_add(self.timeout)
            }
        }
    }

    pub(crate) fn sample(&self) -> Option<QualitySample> {
        self.sample
    }

    pub(crate) fn quality_changed(&mut self, grade: QualityGrade, basis: QualityBasis) -> bool {
        self.quality_change_gate.observe(grade, basis)
    }
}

fn protocol_error(message: &'static str) -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, message)
}
