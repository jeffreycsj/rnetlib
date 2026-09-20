//! Game-runtime lifecycle operations; application shutdown never needs the transport facade.

use crate::runtime::GameRuntime;
use rnet_core::{ErrorCode, Handle, Result};
use std::time::Duration;

impl GameRuntime {
    /// Closes one game session while leaving its listener and other players active.
    pub fn close_session(&self, session: Handle) -> Result<()> {
        self.network.close_session(session, ErrorCode::Cancelled)?;
        self.forget_session(session);
        self.forget_quality_session(session);
        Ok(())
    }

    /// Stops the runtime after the requested drain period and closes remaining endpoints.
    pub fn stop(&self, drain_timeout: Duration) -> Result<()> {
        self.network.stop(drain_timeout)?;
        self.heartbeat_trackers
            .lock()
            .expect("heartbeat table poisoned")
            .clear();
        self.udp_sessions
            .lock()
            .expect("UDP quality table poisoned")
            .clear();
        self.endpoint_transports
            .lock()
            .expect("game endpoint table poisoned")
            .clear();
        Ok(())
    }
}
