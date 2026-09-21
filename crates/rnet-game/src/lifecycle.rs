//! Game-runtime lifecycle operations; application shutdown never needs the transport facade.

use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Handle, Result};
use std::time::Duration;

impl GameRuntime {
    /// Closes one game session while leaving its listener and other players active.
    pub fn close_session(&self, session: Handle) -> Result<()> {
        // Serialize with readiness publication so a racing v4 control cannot resurrect a
        // locally closed handle after cleanup.
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        self.revoke_resume_session(session);
        self.network.close_session(session, ErrorCode::Cancelled)?;
        self.forget_ready_session(session);
        Ok(())
    }

    /// Stops the runtime after the requested drain period and closes remaining endpoints.
    pub fn stop(&self, drain_timeout: Duration) -> Result<()> {
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        self.network.stop(drain_timeout)?;
        *self.range.lock().expect("range state poisoned") = Default::default();
        self.heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .clear();
        self.clock_trackers
            .lock()
            .expect("clock table poisoned")
            .clear();
        self.ready_sessions
            .write()
            .expect("game ready table poisoned")
            .clear();
        self.udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .clear();
        self.realtime
            .lock()
            .expect("realtime queue poisoned")
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
