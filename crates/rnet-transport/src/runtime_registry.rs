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
        self.shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .get_mut(endpoint)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid endpoint"))?
            .abort = Some(abort);
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
                queued_bytes: ByteBudget::new(self.shared.config.max_session_queued_bytes),
            })
    }

    pub(crate) fn insert_pending_session(
        &self,
        endpoint: Handle,
        target: SessionTarget,
        auth_decision: Option<oneshot::Sender<bool>>,
    ) -> Result<Handle> {
        crate::state::insert_session_route(
            &self.shared,
            SessionRoute {
                endpoint,
                target,
                established: false,
                auth_decision,
                security_commands: None,
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
            release_pending_session(&self.shared, &route);
        }
    }
}
