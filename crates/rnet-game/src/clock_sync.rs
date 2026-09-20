//! Four-timestamp game clock controls. All timestamps use a runtime-local monotonic origin.

use crate::envelope::{encode_control, ControlKind};
use bytes::Bytes;
use rnet_core::{ErrorCode, Result, RnetError};
use std::time::{Duration, Instant};

const PROBE: u8 = 1;
const REPLY: u8 = 2;
pub(crate) const PROBE_LEN: usize = 17;
pub(crate) const REPLY_LEN: usize = 33;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClockPacket {
    Probe {
        nonce: u64,
        t1_us: u64,
    },
    Reply {
        nonce: u64,
        t1_us: u64,
        t2_us: u64,
        t3_us: u64,
    },
}

/// The signed offset maps client runtime-local microseconds into server runtime-local
/// microseconds. It is an approximate transport sample, not UTC or a trusted game clock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockSyncSample {
    pub server_minus_client_us: i64,
    pub rtt: Duration,
    pub samples: u64,
}

pub(crate) fn encode_clock(packet: ClockPacket, maximum_len: usize) -> Result<Bytes> {
    // The envelope copies once into its bounded output; avoid a second heap allocation on
    // every periodic probe and reply.
    let mut body = [0_u8; REPLY_LEN];
    let len = match packet {
        ClockPacket::Probe { nonce, t1_us } => {
            body[0] = PROBE;
            body[1..9].copy_from_slice(&nonce.to_be_bytes());
            body[9..17].copy_from_slice(&t1_us.to_be_bytes());
            PROBE_LEN
        }
        ClockPacket::Reply {
            nonce,
            t1_us,
            t2_us,
            t3_us,
        } => {
            body[0] = REPLY;
            for (index, timestamp) in [nonce, t1_us, t2_us, t3_us].iter().enumerate() {
                body[1 + index * 8..9 + index * 8].copy_from_slice(&timestamp.to_be_bytes());
            }
            REPLY_LEN
        }
    };
    encode_control(ControlKind::ClockSync, &body[..len], maximum_len)
}

pub(crate) fn decode_clock(payload: &[u8]) -> Result<ClockPacket> {
    let invalid = || RnetError::new(ErrorCode::ProtocolError, "invalid clock sync control");
    let word = |start: usize| -> u64 {
        u64::from_be_bytes(
            payload[start..start + 8]
                .try_into()
                .expect("validated length"),
        )
    };
    match payload.first().copied() {
        Some(PROBE) if payload.len() == PROBE_LEN => Ok(ClockPacket::Probe {
            nonce: word(1),
            t1_us: word(9),
        }),
        Some(REPLY) if payload.len() == REPLY_LEN => Ok(ClockPacket::Reply {
            nonce: word(1),
            t1_us: word(9),
            t2_us: word(17),
            t3_us: word(25),
        }),
        _ => Err(invalid()),
    }
}

/// Uses wide signed intermediates because untrusted peer timestamps can span the u64 range.
pub(crate) fn calculate_sample(
    t1_us: u64,
    t2_us: u64,
    t3_us: u64,
    t4_us: u64,
) -> Option<ClockSyncSample> {
    if t3_us < t2_us || t4_us < t1_us {
        return None;
    }
    let client_elapsed = t4_us - t1_us;
    let server_elapsed = t3_us - t2_us;
    let rtt_us = client_elapsed.checked_sub(server_elapsed)?;
    let offset =
        ((i128::from(t2_us) - i128::from(t1_us)) + (i128::from(t3_us) - i128::from(t4_us))) / 2;
    Some(ClockSyncSample {
        server_minus_client_us: i64::try_from(offset).ok()?,
        rtt: Duration::from_micros(rtt_us),
        samples: 1,
    })
}

/// One outstanding client request and one reply-rate gate per ready session.
#[derive(Debug, Default)]
pub(crate) struct ClockSyncTracker {
    client: bool,
    last_sent: Option<Instant>,
    pending: Option<(u64, u64, Instant)>,
    last_inbound_probe: Option<Instant>,
    sample: Option<ClockSyncSample>,
}

impl ClockSyncTracker {
    pub(crate) fn new_client() -> Self {
        Self {
            client: true,
            ..Self::default()
        }
    }

    pub(crate) fn new_server() -> Self {
        Self::default()
    }

    pub(crate) fn is_client(&self) -> bool {
        self.client
    }

    pub(crate) fn should_probe(
        &mut self,
        now: Instant,
        interval: Duration,
        timeout: Duration,
    ) -> bool {
        if !self.client {
            return false;
        }
        if let Some((_, _, sent_at)) = self.pending {
            if now.saturating_duration_since(sent_at) < timeout {
                return false;
            }
            self.pending = None;
        }
        self.last_sent
            .is_none_or(|last| now.saturating_duration_since(last) >= interval)
    }

    pub(crate) fn mark_sent(&mut self, nonce: u64, t1_us: u64, now: Instant) {
        self.last_sent = Some(now);
        self.pending = Some((nonce, t1_us, now));
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn accept_reply(
        &mut self,
        nonce: u64,
        t1_us: u64,
        t2_us: u64,
        t3_us: u64,
        t4_us: u64,
        received_at: Instant,
        timeout: Duration,
    ) -> Option<ClockSyncSample> {
        if !self.client {
            return None;
        }
        let (expected_nonce, expected_t1, sent_at) = self.pending?;
        if nonce != expected_nonce
            || t1_us != expected_t1
            || received_at
                .checked_duration_since(sent_at)
                .is_none_or(|age| age > timeout)
        {
            return None;
        }
        let mut sample = calculate_sample(t1_us, t2_us, t3_us, t4_us)?;
        sample.samples = self
            .sample
            .map_or(1, |previous| previous.samples.saturating_add(1));
        self.sample = Some(sample);
        self.last_sent = Some(received_at);
        self.pending = None;
        Some(sample)
    }

    pub(crate) fn admit_probe(&mut self, received_at: Instant, interval: Duration) -> bool {
        if self.client {
            return false;
        }
        let spacing = (interval / 4).max(Duration::from_millis(1));
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

    pub(crate) fn sample(&self) -> Option<ClockSyncSample> {
        self.sample
    }
}
