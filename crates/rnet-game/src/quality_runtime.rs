//! Per-session UDP sequence accounting. A failed enqueue never consumes a wire sequence.

use crate::envelope::{encode_application, encode_udp_application};
use crate::quality::{UdpLossSnapshot, UdpSessionQuality};
use crate::runtime::{GameRuntime, GameSendOptions};
use rnet_core::{ErrorCode, Handle, Result, RnetError, Transport};
use rnet_transport::KcpRetransmissionSnapshot;
use std::sync::{Arc, Mutex};

impl GameRuntime {
    /// Returns per-session KCP PUSH retransmissions without waiting for a heartbeat sample.
    /// TCP/UDP return `None`; an unknown or not-yet-ready game session is rejected.
    pub fn kcp_retransmission_snapshot(
        &self,
        session: Handle,
    ) -> Result<Option<KcpRetransmissionSnapshot>> {
        if !self
            .heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .contains_key(&session)
        {
            return Err(RnetError::new(
                ErrorCode::InvalidHandle,
                "game session is not ready",
            ));
        }
        self.network.kcp_retransmission_snapshot(session)
    }

    /// Returns a receiver-side UDP data-gap estimate; TCP and KCP return `None`.
    /// An unknown or not-yet-ready game session returns `InvalidHandle`.
    pub fn udp_loss_snapshot(&self, session: Handle) -> Result<Option<UdpLossSnapshot>> {
        let udp = self
            .udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .get(&session)
            .cloned();
        if let Some(udp) = udp {
            return Ok(Some(
                udp.lock()
                    .expect("UDP quality state poisoned")
                    .inbound
                    .snapshot(),
            ));
        }
        if self
            .heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .contains_key(&session)
        {
            Ok(None)
        } else {
            Err(RnetError::new(
                ErrorCode::InvalidHandle,
                "game session is not ready",
            ))
        }
    }

    pub(crate) fn track_quality_session(&self, session: Handle, transport: Transport) {
        if transport == Transport::Udp {
            self.udp_sessions
                .lock()
                .expect("UDP quality table poisoned")
                .insert(session, Arc::new(Mutex::new(UdpSessionQuality::default())));
        }
    }

    pub(crate) fn forget_quality_session(&self, session: Handle) {
        self.udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .remove(&session);
    }

    pub(crate) fn send_game_application(
        &self,
        session: Handle,
        payload: &[u8],
        options: GameSendOptions,
    ) -> Result<()> {
        self.ensure_game_ready(session)?;
        let udp = self
            .udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .get(&session)
            .cloned();
        if let Some(udp) = udp {
            // Serialize only sends for this UDP session. Advancing after a successful enqueue
            // prevents local WouldBlock or size errors from looking like lost wire datagrams.
            let mut state = udp.lock().expect("UDP quality state poisoned");
            let envelope = encode_udp_application(
                payload,
                options.sequence,
                options.tick,
                state.next_outbound_sequence,
                self.maximum_envelope_len,
            )?;
            self.network.send_payload(session, &envelope)?;
            state.next_outbound_sequence = state.next_outbound_sequence.wrapping_add(1);
            return Ok(());
        }
        let envelope = encode_application(
            payload,
            options.sequence,
            options.tick,
            self.maximum_envelope_len,
        )?;
        self.network.send_payload(session, &envelope)
    }

    /// Coalesced snapshots keep their key until the adaptive transport worker takes the slot.
    /// UDP retains its datagram sequence path; TCP/KCP may replace still-pending frames.
    pub(crate) fn send_game_latest_application(
        &self,
        session: Handle,
        key: u64,
        payload: &[u8],
        tick: Option<u32>,
    ) -> Result<()> {
        if self
            .udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .contains_key(&session)
        {
            return self.send_game_application(
                session,
                payload,
                crate::runtime::GameSendOptions {
                    sequence: None,
                    tick,
                },
            );
        }
        self.ensure_game_ready(session)?;
        let envelope = encode_application(payload, None, tick, self.maximum_envelope_len)?;
        self.network.send_payload_latest(session, key, &envelope)?;
        Ok(())
    }

    pub(crate) fn observe_datagram_sequence(
        &self,
        session: Handle,
        sequence: Option<u32>,
    ) -> Result<()> {
        let udp = self
            .udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .get(&session)
            .cloned();
        match (udp, sequence) {
            (Some(udp), Some(sequence)) => {
                udp.lock()
                    .expect("UDP quality state poisoned")
                    .inbound
                    .observe(sequence);
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(RnetError::new(
                ErrorCode::ProtocolError,
                "game data has an invalid transport-owned sequence",
            )),
        }
    }
}
