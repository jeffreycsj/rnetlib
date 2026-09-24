//! Endpoint policy and game-state cleanup independent of bounded lifecycle notifications.

use crate::{GameProtocol, GameRuntime};
use rnet_core::{Handle, Result, Transport};
use std::collections::HashMap;

impl GameRuntime {
    /// Closes an endpoint and releases its game protocol policy.
    pub fn close_endpoint(&self, endpoint: Handle) -> Result<()> {
        let _poll = self.poll_guard.lock().expect("game poll lock poisoned");
        let mut protocols = self
            .server_protocols
            .lock()
            .expect("game protocol table poisoned");
        let mut transports = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned");
        self.network.close_endpoint(endpoint)?;
        self.forget_endpoint_state(endpoint, &mut protocols, &mut transports);
        Ok(())
    }

    pub(crate) fn reap_closed_endpoints(&self) {
        // Registration/cleanup use the same map lock order as listen. While locked, all game
        // endpoints are accounted for; a smaller native count proves some ownership is stale.
        // The healthy path avoids walking every endpoint on every high-frequency game poll.
        let mut protocols = self
            .server_protocols
            .lock()
            .expect("game protocol table poisoned");
        let mut transports = self
            .endpoint_transports
            .lock()
            .expect("game endpoint table poisoned");
        if self.network.metrics_snapshot().current_endpoints >= transports.len() as u64 {
            return;
        }
        let expired: Vec<_> = transports
            .keys()
            .copied()
            .filter(|endpoint| self.network.endpoint_local_addr(*endpoint).is_err())
            .collect();
        for endpoint in expired {
            self.forget_endpoint_state(endpoint, &mut protocols, &mut transports);
        }
    }

    fn forget_endpoint_state(
        &self,
        endpoint: Handle,
        protocols: &mut HashMap<Handle, GameProtocol>,
        transports: &mut HashMap<Handle, Transport>,
    ) {
        self.resume
            .lock()
            .expect("resume state poisoned")
            .tickets
            .revoke_endpoint(endpoint);
        // Also includes sessions awaiting business authorization, not only ready players.
        let sessions: Vec<_> = self
            .session_endpoints
            .lock()
            .expect("game session table poisoned")
            .iter()
            .filter_map(|(session, owner)| (*owner == endpoint).then_some(*session))
            .collect();
        for session in sessions {
            self.revoke_resume_session(session);
            self.forget_ready_session(session);
        }
        self.resume
            .lock()
            .expect("resume state poisoned")
            .client_endpoints
            .remove(&endpoint);
        protocols.remove(&endpoint);
        transports.remove(&endpoint);
        self.range
            .lock()
            .expect("range state poisoned")
            .forget_endpoint(endpoint);
    }
}
