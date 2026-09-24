//! Fatal endpoint-owner exit: registry state is authoritative even if notifications are rejected.

use crate::session_close::{diagnostic_bytes, fail_endpoint_session};
use crate::state::{publish_lifecycle, session_event, Shared};
use rnet_core::{EventType, Handle, RnetError};
use std::sync::Arc;

/// Called synchronously by the owning task immediately before it exits, not to cancel a live task.
/// Holding the endpoint lock through cleanup prevents observers from seeing an invalid endpoint
/// with still-live routes. Lock order is endpoint table, then session table.
pub(crate) fn fail_endpoint(
    shared: &Arc<Shared>,
    endpoint: Handle,
    initial_client: Option<Handle>,
    phase: &'static str,
    error: RnetError,
) {
    let mut endpoints = shared.endpoints.lock().expect("endpoint table poisoned");
    let Some(_record) = endpoints.remove(endpoint) else {
        return;
    };
    let sessions: Vec<_> = {
        let routes = shared.sessions.lock().expect("session table poisoned");
        routes
            .handles()
            .filter(|session| {
                routes
                    .get(*session)
                    .is_some_and(|route| route.endpoint == endpoint)
            })
            .collect()
    };
    let mut event = session_event(EventType::EndpointError, endpoint, 0);
    event.status = error.code();
    event.data = diagnostic_bytes(phase, &error);
    publish_lifecycle(shared, event);
    for session in sessions {
        fail_endpoint_session(
            shared,
            session,
            phase,
            &error,
            initial_client == Some(session),
        );
    }
}

#[cfg(test)]
#[path = "endpoint_failure_tests.rs"]
mod tests;
