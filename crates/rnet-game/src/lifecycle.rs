//! Game-runtime lifecycle operations; application shutdown never needs the transport facade.

use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use std::time::{Duration, Instant};

impl GameRuntime {
    /// Closes one game session while leaving its listener and other players active.
    /// `session` is a local handle. Revokes its resume credentials and removes pending game
    /// sends; use this for an explicit kick, not for preserving reconnect eligibility.
    /// Returns an error for an unknown/closed handle. Serializes with poll and readiness
    /// publication; shutdown of a client connection may also retire its dedicated endpoint.
    pub fn close_session(&self, session: Handle) -> Result<()> {
        // Serialize with readiness publication so a racing v4 control cannot resurrect a
        // locally closed handle after cleanup.
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        self.revoke_resume_session(session);
        self.network.close_session(session, ErrorCode::Cancelled)?;
        self.forget_ready_session(session);
        Ok(())
    }

    /// Rejects new game sends, attempts draining, then closes remaining endpoints.
    /// `drain_timeout` is a best-effort local drain budget; zero skips the grace period.
    /// It does not guarantee remote delivery or bound time spent waiting for concurrent poll.
    /// Pending work left after draining is discarded, and all resume tickets are invalidated.
    /// An unrepresentable deadline returns `InvalidArgument` before shutdown begins.
    /// Do not invoke shutdown from this runtime's logger callback.
    pub fn stop(&self, drain_timeout: Duration) -> Result<()> {
        let deadline = Instant::now().checked_add(drain_timeout).ok_or_else(|| {
            RnetError::new(
                ErrorCode::InvalidArgument,
                "game stop timeout cannot form a deadline",
            )
        })?;
        // Reject new queue admission before taking the poll lock. Existing admitted work is then
        // either drained within the deadline or counted as a shutdown drop during queue cleanup.
        self.admission.begin_stop();
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let _registration = self
            .endpoint_registration
            .lock()
            .expect("endpoint registration poisoned");
        while (self.scheduled_queue_snapshot().queued_messages != 0
            || self.realtime_queue_snapshot().queued_messages != 0)
            && Instant::now() < deadline
        {
            let scheduled = self.flush_scheduled_inner(self.scheduled_flush_batch);
            let realtime = self.flush_realtime_inner(self.realtime_flush_batch);
            if scheduled == 0 && realtime == 0 {
                std::thread::yield_now();
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        self.network.stop(remaining)?;
        self.range.lock().expect("range state poisoned").clear();
        self.heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .clear();
        self.clock_trackers
            .lock()
            .expect("clock table poisoned")
            .clear();
        self.admission.clear();
        self.udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .clear();
        self.realtime
            .lock()
            .expect("realtime queue poisoned")
            .clear();
        self.scheduled
            .lock()
            .expect("scheduled queue poisoned")
            .clear();
        self.endpoint_transports
            .lock()
            .expect("game endpoint table poisoned")
            .clear();
        self.session_endpoints
            .lock()
            .expect("game session table poisoned")
            .clear();
        let mut resume = self.resume.lock().expect("resume state poisoned");
        resume.tickets.clear();
        resume.server_sessions.clear();
        resume.pending_server.clear();
        resume.inflight_server.clear();
        resume.revoked_inflight.clear();
        resume.client_endpoints.clear();
        Ok(())
    }
}
