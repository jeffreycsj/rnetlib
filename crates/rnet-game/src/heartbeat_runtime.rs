//! Poll-driven heartbeat scheduling over transport-authenticated game controls.

use crate::envelope::{decode, ControlKind, DecodedEnvelope};
use crate::event::NetworkQuality;
use crate::heartbeat::{
    decode_heartbeat, encode_heartbeat, HeartbeatKind, HeartbeatPacket, HeartbeatTracker,
    QualitySample,
};
use crate::runtime::GameRuntime;
use ring::rand::{SecureRandom, SystemRandom};
use rnet_core::{ErrorCode, Event, Handle, Result, RnetError};
use std::time::Instant;

impl GameRuntime {
    /// Returns the latest authenticated sample, or `None` before the first matching reply.
    pub fn network_quality(&self, session: Handle) -> Result<Option<NetworkQuality>> {
        let trackers = self
            .heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned");
        let tracker = trackers
            .get(&session)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "game session is not ready"))?;
        Ok(tracker.sample().map(NetworkQuality::from))
    }

    pub(crate) fn track_session(&self, session: Handle) {
        let tracker = HeartbeatTracker::new(
            self.heartbeat_interval,
            self.heartbeat_timeout,
            Instant::now(),
        )
        .expect("validated heartbeat policy");
        self.heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .insert(session, tracker);
    }

    pub(crate) fn forget_session(&self, session: Handle) {
        self.heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .remove(&session);
    }

    pub(crate) fn handle_heartbeat_event(&self, event: &Event) -> Result<Option<crate::GameEvent>> {
        let DecodedEnvelope::Control {
            kind: ControlKind::Heartbeat,
            payload,
        } = decode(&event.data, self.maximum_envelope_len)?
        else {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "authenticated game control is not a heartbeat",
            ));
        };
        let packet = decode_heartbeat(&payload)?;
        if !self
            .heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .contains_key(&event.session)
        {
            // A locally closed session can still have an authenticated control already queued.
            // Transport accepts these controls only for established sessions, so no tracker now
            // means the game session has since left the ready state.
            return Ok(None);
        }
        match packet.kind {
            HeartbeatKind::Probe => {
                let admitted = self
                    .heartbeat_trackers
                    .lock()
                    .expect("heartbeat table poisoned")
                    .get_mut(&event.session)
                    .is_some_and(|tracker| tracker.admit_probe(event.queued_at));
                if !admitted {
                    return Ok(None);
                }
                let ack = encode_heartbeat(
                    HeartbeatPacket {
                        kind: HeartbeatKind::Ack,
                        challenge: packet.challenge,
                    },
                    self.maximum_envelope_len,
                )?;
                // A full bounded queue may drop this reply; the probing peer will time out.
                // Transport send failures are not malformed peer input.
                let _ = self.network.send_game_control(event.session, &ack);
            }
            HeartbeatKind::Ack => {
                let mut trackers = self
                    .heartbeat_trackers
                    .lock()
                    .expect("heartbeat table poisoned");
                if let Some(tracker) = trackers.get_mut(&event.session) {
                    tracker.accept_ack(packet.challenge, event.queued_at, Instant::now());
                }
            }
        }
        Ok(None)
    }

    pub(crate) fn drive_heartbeats(&self) {
        let now = Instant::now();
        let random = SystemRandom::new();
        let mut close = Vec::new();
        let mut trackers = self
            .heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned");
        for (&session, tracker) in trackers.iter_mut() {
            // Allow one poll tick for a timely acknowledgement that I/O enqueued concurrently
            // with the empty-queue check. The packet's recorded arrival time still decides
            // whether it met the configured deadline.
            if now
                .checked_sub(std::time::Duration::from_millis(50))
                .is_some_and(|earlier| tracker.timed_out(earlier))
            {
                close.push((session, ErrorCode::Timeout));
                continue;
            }
            if !tracker.should_probe(now) {
                continue;
            }
            let mut challenge = [0_u8; 8];
            if random.fill(&mut challenge).is_err() {
                close.push((session, ErrorCode::CryptoError));
                continue;
            }
            let challenge = u64::from_be_bytes(challenge);
            let encoded = match encode_heartbeat(
                HeartbeatPacket {
                    kind: HeartbeatKind::Probe,
                    challenge,
                },
                self.maximum_envelope_len,
            ) {
                Ok(encoded) => encoded,
                Err(_) => {
                    close.push((session, ErrorCode::MessageTooLarge));
                    continue;
                }
            };
            if self.network.send_game_control(session, &encoded).is_ok() {
                let _ = tracker.mark_sent(challenge, now);
            }
        }
        drop(trackers);
        for (session, reason) in close {
            let _ = self.network.close_session(session, reason);
            self.forget_session(session);
        }
    }
}

impl From<QualitySample> for NetworkQuality {
    fn from(sample: QualitySample) -> Self {
        Self {
            last_rtt: sample.last_rtt,
            smoothed_rtt: sample.smoothed_rtt,
            jitter: sample.jitter,
            samples: sample.samples,
        }
    }
}
