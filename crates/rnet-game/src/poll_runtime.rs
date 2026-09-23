//! Poll scheduling and transport-to-game event batching.

use crate::event::GameEvent;
use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Event, EventType};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const NONBLOCKING_EVENT_BATCH: usize = 64;
const MAX_INTERNAL_EVENTS_PER_POLL: usize = 1024;

impl GameRuntime {
    /// Polls typed game events. Unsupported internal controls fail closed until implemented.
    pub fn poll(&self, capacity: usize, timeout: Duration) -> Vec<GameEvent> {
        // Game controls are handled by the same event queue as lifecycle changes. Serializing
        // polls preserves their order and keeps challenge state single-writer.
        let _guard = self.poll_guard.lock().expect("game poll lock poisoned");
        self.maybe_log_metrics();
        self.flush_scheduled_inner(self.scheduled_flush_batch);
        self.flush_realtime_inner(self.realtime_flush_batch);
        let mut carried = Vec::new();
        {
            let mut range = self.range.lock().expect("range state poisoned");
            range.drain_completed(&mut carried, capacity);
        }
        let mut internal_events = 0usize;
        if capacity == 0 || !carried.is_empty() {
            // Public backlog must not hide authenticated acknowledgements or negotiation controls.
            // Any newly converted public event remains ordered in the completed queue when the
            // caller has no remaining output capacity.
            self.drain_network_nonblocking(&mut carried, capacity, &mut internal_events);
            self.drive_heartbeats();
            self.drive_clock_sync();
            self.drive_range_negotiation();
            return carried;
        }
        let deadline = Instant::now().checked_add(timeout);
        loop {
            // Authenticated replies already queued by I/O workers take precedence over timeout.
            // Drain even when a batch contains only internal controls, so a small public capacity
            // cannot leave a timely acknowledgement stranded behind other control events.
            let mut output = Vec::new();
            self.drain_network_nonblocking(&mut output, capacity, &mut internal_events);
            if internal_events != 0 {
                // Hidden controls and public events alike must not defer liveness scheduling.
                // Conversion runs first so already queued authenticated acknowledgements win.
                self.drive_heartbeats();
                self.drive_clock_sync();
                self.drive_range_negotiation();
                if !output.is_empty() {
                    return output;
                }
                // A malicious authenticated peer cannot keep one poll call trapped forever by
                // continuously filling the queue with controls that are hidden from game code.
                if internal_events >= MAX_INTERNAL_EVENTS_PER_POLL {
                    return Vec::new();
                }
            }
            self.drive_heartbeats();
            self.drive_clock_sync();
            self.drive_range_negotiation();
            let remaining = deadline
                .map(|deadline| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(timeout);
            let wait = if self
                .heartbeat_trackers
                .lock()
                .expect("heartbeat table poisoned")
                .is_empty()
            {
                remaining
            } else {
                remaining.min(Duration::from_millis(50))
            };
            let events = self.network.poll_events(capacity, wait);
            internal_events = internal_events.saturating_add(events.len());
            let mut output = Vec::new();
            self.convert_events_into(events, &mut output, capacity);
            self.drive_heartbeats();
            self.drive_clock_sync();
            self.drive_range_negotiation();
            if !output.is_empty()
                || timeout.is_zero()
                || deadline.is_some_and(|deadline| Instant::now() >= deadline)
                || internal_events >= MAX_INTERNAL_EVENTS_PER_POLL
            {
                return output;
            }
        }
    }

    fn drain_network_nonblocking(
        &self,
        output: &mut Vec<GameEvent>,
        capacity: usize,
        internal_events: &mut usize,
    ) {
        while *internal_events < MAX_INTERNAL_EVENTS_PER_POLL {
            let remaining_output = capacity.saturating_sub(output.len());
            let batch = if remaining_output == 0 {
                1
            } else {
                (MAX_INTERNAL_EVENTS_PER_POLL - *internal_events)
                    .min(NONBLOCKING_EVENT_BATCH)
                    .min(remaining_output)
            };
            let events = self.network.poll_events(batch, Duration::ZERO);
            if events.is_empty() {
                break;
            }
            *internal_events = (*internal_events).saturating_add(events.len());
            if self.convert_events_into(events, output, capacity) {
                break;
            }
        }
    }

    fn convert_events_into(
        &self,
        events: Vec<Event>,
        output: &mut Vec<GameEvent>,
        capacity: usize,
    ) -> bool {
        let mut deferred_public = false;
        for event in events {
            let endpoint = event.endpoint;
            let session = event.session;
            let is_business_message = event.event_type == EventType::Message;
            let integrity_verified = event.integrity_verified;
            // These lifecycle payloads are local diagnostics, unlike opaque application bytes
            // and authentication tickets. Preserve the cause before typed conversion drops it.
            let detail = (matches!(
                event.event_type,
                EventType::EndpointError | EventType::JoinFailed | EventType::SessionClosed
            ) && !event.data.is_empty())
            .then(|| String::from_utf8_lossy(&event.data).into_owned());
            match self.convert_event(event) {
                Ok(Some(event)) => {
                    self.log_game_event_detail(&event, detail.as_deref());
                    deferred_public |= self
                        .range
                        .lock()
                        .expect("range state poisoned")
                        .queue_public(event, output, capacity);
                }
                Ok(None) => {}
                Err(error) => {
                    let unverified_plaintext = self.allow_plaintext_business_data
                        && is_business_message
                        && !integrity_verified;
                    if unverified_plaintext {
                        // An on-path sender controls both contents and rate in plaintext mode.
                        // Keep the session usable and expose only a low-cardinality counter.
                        self.protocol_metrics
                            .plaintext_invalid_envelopes_dropped
                            .fetch_add(1, Ordering::Relaxed);
                    } else {
                        if session != 0 {
                            let _ = self
                                .network
                                .close_session(session, ErrorCode::ProtocolError);
                        }
                        let violation = GameEvent::ProtocolViolation { endpoint, session };
                        self.log_game_event_detail(&violation, Some(&error.to_string()));
                        deferred_public |= self
                            .range
                            .lock()
                            .expect("range state poisoned")
                            .queue_public(violation, output, capacity);
                    }
                }
            }
            self.range
                .lock()
                .expect("range state poisoned")
                .drain_completed(output, capacity);
        }
        deferred_public
    }
}
