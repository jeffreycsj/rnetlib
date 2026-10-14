//! Business-envelope validation is separate from local admission and negotiation failures.

use crate::envelope::{decode, DecodedEnvelope};
use crate::event::{GameEvent, GameMessage};
use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Event, Result, RnetError};
use std::sync::atomic::Ordering;

impl GameRuntime {
    pub(crate) fn convert_business_event(&self, event: &Event) -> Result<Option<GameEvent>> {
        let message = match self.decode_business_message(event) {
            Ok(message) => message,
            Err(_) if !event.integrity_verified => {
                // Trust integrity captured at receive time, not the current security mode or
                // this runtime's server policy: the frame may predate an encryption switch.
                // Unauthenticated corruption is counted, not allowed to flood the game loop.
                self.protocol_metrics
                    .plaintext_invalid_envelopes_dropped
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        // Valid messages can still exceed local pre-ready budgets. Such failures must remain
        // visible in both modes; they are not evidence of malformed plaintext on the wire.
        self.buffer_range_message(message)
    }

    fn decode_business_message(&self, event: &Event) -> Result<GameMessage> {
        if event.status != ErrorCode::Ok {
            return Err(RnetError::new(
                ErrorCode::ProtocolError,
                "game message carries an unsuccessful transport status",
            ));
        }
        match decode(&event.data, self.maximum_envelope_len)? {
            DecodedEnvelope::Application {
                sequence,
                tick,
                datagram_sequence,
                payload,
            } => {
                self.observe_datagram_sequence(event.session, datagram_sequence)?;
                Ok(GameMessage {
                    endpoint: event.endpoint,
                    session: event.session,
                    sequence,
                    tick,
                    correlation_id: event.correlation_id,
                    payload,
                })
            }
            DecodedEnvelope::Control { .. } => Err(RnetError::new(
                ErrorCode::ProtocolError,
                "game control envelope received on the business channel",
            )),
        }
    }
}
