//! Protected, bounded clock sampling for ready game sessions.

use crate::clock_sync::{
    decode_clock, encode_clock, ClockPacket, ClockSyncSample, ClockSyncTracker,
};
use crate::event::GameEvent;
use crate::runtime::GameRuntime;
use ring::rand::{SecureRandom, SystemRandom};
use rnet_core::{ErrorCode, Event, Handle, Result, RnetError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct ClockSyncMetrics {
    pub(crate) probes_sent: AtomicU64,
    pub(crate) replies_sent: AtomicU64,
    pub(crate) samples: AtomicU64,
    pub(crate) rejected: AtomicU64,
    pub(crate) send_failures: AtomicU64,
}

/// Aggregate counters without per-player labels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClockSyncMetricsSnapshot {
    pub probes_sent: u64,
    pub replies_sent: u64,
    pub samples: u64,
    pub rejected: u64,
    pub send_failures: u64,
}

impl GameRuntime {
    /// Microseconds since this runtime's monotonic origin; never a wall-clock timestamp.
    pub fn clock_micros(&self) -> u64 {
        self.clock_at(Instant::now())
    }

    pub(crate) fn clock_at(&self, instant: Instant) -> u64 {
        u64::try_from(
            instant
                .saturating_duration_since(self.clock_origin)
                .as_micros(),
        )
        .unwrap_or(u64::MAX)
    }

    /// Most recent client-side sample. Server sessions do not estimate an offset.
    pub fn clock_sync_snapshot(&self, session: Handle) -> Result<Option<ClockSyncSample>> {
        self.ensure_game_ready(session)?;
        Ok(self
            .clock_trackers
            .lock()
            .expect("clock table poisoned")
            .get(&session)
            .and_then(ClockSyncTracker::sample))
    }

    pub fn clock_sync_metrics_snapshot(&self) -> ClockSyncMetricsSnapshot {
        let metrics = &self.clock_metrics;
        ClockSyncMetricsSnapshot {
            probes_sent: metrics.probes_sent.load(Ordering::Relaxed),
            replies_sent: metrics.replies_sent.load(Ordering::Relaxed),
            samples: metrics.samples.load(Ordering::Relaxed),
            rejected: metrics.rejected.load(Ordering::Relaxed),
            send_failures: metrics.send_failures.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn track_clock_session(&self, session: Handle, client: bool) {
        self.clock_trackers
            .lock()
            .expect("clock table poisoned")
            .insert(
                session,
                if client {
                    ClockSyncTracker::new_client()
                } else {
                    ClockSyncTracker::new_server()
                },
            );
    }

    pub(crate) fn forget_clock_session(&self, session: Handle) {
        self.clock_trackers
            .lock()
            .expect("clock table poisoned")
            .remove(&session);
    }

    pub(crate) fn drive_clock_sync(&self) {
        let now = Instant::now();
        let spacing = (self.heartbeat_interval / 4)
            .clamp(Duration::from_millis(1), Duration::from_millis(250));
        {
            let mut next = self.clock_next_scan.lock().expect("clock scan poisoned");
            if now < *next {
                return;
            }
            *next = now + spacing;
        }
        let random = SystemRandom::new();
        let mut trackers = self.clock_trackers.lock().expect("clock table poisoned");
        for (&session, tracker) in trackers.iter_mut() {
            if !tracker.should_probe(now, self.heartbeat_interval, self.heartbeat_timeout) {
                continue;
            }
            let mut nonce = [0_u8; 8];
            if random.fill(&mut nonce).is_err() {
                self.clock_metrics
                    .send_failures
                    .fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let nonce = u64::from_be_bytes(nonce);
            let t1_us = self.clock_at(Instant::now());
            let encoded = encode_clock(
                ClockPacket::Probe { nonce, t1_us },
                self.maximum_envelope_len,
            );
            if encoded
                .as_ref()
                .is_ok_and(|bytes| self.network.send_game_control(session, bytes).is_ok())
            {
                tracker.mark_sent(nonce, t1_us, now);
                self.clock_metrics
                    .probes_sent
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                self.clock_metrics
                    .send_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn handle_clock_sync_event(
        &self,
        event: &Event,
        payload: &[u8],
    ) -> Result<Option<GameEvent>> {
        if self.ensure_game_ready(event.session).is_err() {
            return Ok(None);
        }
        let packet = decode_clock(payload)?;
        let mut trackers = self.clock_trackers.lock().expect("clock table poisoned");
        let Some(tracker) = trackers.get_mut(&event.session) else {
            return Ok(None);
        };
        match packet {
            ClockPacket::Probe { nonce, t1_us } if !tracker.is_client() => {
                if !tracker.admit_probe(event.queued_at, self.heartbeat_interval) {
                    self.clock_metrics.rejected.fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }
                let t2_us = self.clock_at(event.queued_at);
                let t3_us = self.clock_micros();
                let reply = encode_clock(
                    ClockPacket::Reply {
                        nonce,
                        t1_us,
                        t2_us,
                        t3_us,
                    },
                    self.maximum_envelope_len,
                )?;
                if self
                    .network
                    .send_game_control(event.session, &reply)
                    .is_ok()
                {
                    self.clock_metrics
                        .replies_sent
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    self.clock_metrics
                        .send_failures
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            ClockPacket::Reply {
                nonce,
                t1_us,
                t2_us,
                t3_us,
            } if tracker.is_client() => {
                if tracker
                    .accept_reply(
                        nonce,
                        t1_us,
                        t2_us,
                        t3_us,
                        self.clock_at(event.queued_at),
                        event.queued_at,
                        self.heartbeat_timeout,
                    )
                    .is_some()
                {
                    self.clock_metrics.samples.fetch_add(1, Ordering::Relaxed);
                } else {
                    // A delayed/replayed reply is not grounds to disconnect a healthy session.
                    self.clock_metrics.rejected.fetch_add(1, Ordering::Relaxed);
                }
            }
            _ => {
                return Err(RnetError::new(
                    ErrorCode::ProtocolError,
                    "clock sync control has wrong endpoint role",
                ))
            }
        }
        Ok(None)
    }
}
