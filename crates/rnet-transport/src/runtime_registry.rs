//! Internal endpoint and session registry operations.

use crate::metrics::AdmissionRejectReason;
use crate::runtime::NetworkRuntime;
use crate::state::{
    release_pending_session, ByteBudget, EndpointRecord, SessionRoute, SessionTarget,
};
use rnet_core::{ErrorCode, Event, EventType, Handle, Result, RnetError, Transport};
use std::net::SocketAddr;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

impl NetworkRuntime {
    pub(crate) fn insert_endpoint(
        &self,
        local_addr: SocketAddr,
        transport: Transport,
    ) -> Result<Handle> {
        let mut endpoints = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned");
        if endpoints.len() >= self.shared.config.max_endpoints {
            self.shared
                .metrics
                .record_admission_rejected(AdmissionRejectReason::EndpointLimit);
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "runtime endpoint limit reached",
            ));
        }
        Ok(endpoints.insert(EndpointRecord {
            local_addr,
            transport,
            abort: None,
        }))
    }

    pub(crate) fn set_endpoint_abort(&self, endpoint: Handle, abort: AbortHandle) -> Result<()> {
        let mut endpoints = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned");
        let Some(record) = endpoints.get_mut(endpoint) else {
            // Close/stop can win before task registration. Never leave that task detached.
            abort.abort();
            return Err(RnetError::new(ErrorCode::InvalidHandle, "invalid endpoint"));
        };
        record.abort = Some(abort);
        Ok(())
    }

    pub(crate) fn insert_session(&self, endpoint: Handle, target: SessionTarget) -> Handle {
        self.shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .insert(SessionRoute {
                endpoint,
                target,
                established: true,
                auth_decision: None,
                security_commands: None,
                allows_game_controls: false,
                queued_bytes: ByteBudget::new(self.shared.config.max_session_queued_bytes),
            })
    }

    pub(crate) fn insert_pending_session(
        &self,
        endpoint: Handle,
        target: SessionTarget,
        auth_decision: Option<oneshot::Sender<bool>>,
        allows_game_controls: bool,
    ) -> Result<Handle> {
        crate::state::insert_session_route(
            &self.shared,
            SessionRoute {
                endpoint,
                target,
                established: false,
                auth_decision,
                security_commands: None,
                allows_game_controls,
                queued_bytes: ByteBudget::new(self.shared.config.max_session_queued_bytes),
            },
        )
    }

    pub(crate) fn push_endpoint_opened(&self, endpoint: Handle) -> Result<()> {
        let mut event = Event::simple(EventType::EndpointOpened);
        event.endpoint = endpoint;
        self.shared.events.try_push(event)
    }

    pub(crate) fn discard_endpoint(&self, endpoint: Handle) {
        let _ = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .remove(endpoint);
    }

    pub(crate) fn discard_session(&self, session: Handle) {
        if let Some(route) = self
            .shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .remove(session)
        {
            self.shared.latest.forget_session(session);
            release_pending_session(&self.shared, &route);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{NetworkRuntime, RuntimeConfig};
    use rnet_core::ErrorCode;
    use std::time::Duration;
    use tokio::sync::oneshot;

    #[test]
    fn failed_endpoint_registration_cancels_the_waiting_task() {
        let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
        let (_start, ready) = oneshot::channel::<()>();
        let task = runtime.runtime.spawn(async {
            let _ = ready.await;
        });
        assert_eq!(
            runtime
                .set_endpoint_abort(0, task.abort_handle())
                .unwrap_err()
                .code(),
            ErrorCode::InvalidHandle
        );
        // Keep the task pending: a task already completing may legally win against abort.
        let result = runtime.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .expect("unregistered task was not cancelled")
        });
        assert!(
            result.is_err_and(|error| error.is_cancelled()),
            "unregistered endpoint task survived"
        );
        runtime.stop(Duration::ZERO).unwrap();
    }
}
