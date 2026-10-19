use crate::runtime::NetworkRuntime;
use crate::state::{publish_lifecycle, release_pending_session, session_event, Shared};
use rnet_core::{ErrorCode, Event, EventType, Handle, Lifecycle, Result, RnetError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

impl Drop for NetworkRuntime {
    fn drop(&mut self) {
        let endpoints: Vec<_> = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .handles()
            .collect();
        for endpoint in endpoints {
            let _ = self.close_endpoint(endpoint);
        }
    }
}

impl NetworkRuntime {
    /// Invalidates the listen/connect endpoint and all its session handles, requesting task
    /// cancellation and publishing session-close events. A stale endpoint returns `InvalidHandle`.
    /// Does not wait for task cancellation or guarantee queued bytes reached peers. There is
    /// no endpoint-close acknowledgement event; event delivery is subject to bounded capacity.
    pub fn close_endpoint(&self, endpoint: Handle) -> Result<()> {
        let record = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .remove(endpoint)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid endpoint"))?;
        if let Some(abort) = record.abort {
            abort.abort();
        }
        close_endpoint_sessions(&self.shared, endpoint);
        Ok(())
    }

    /// Invalidates one session locally without closing its listener or other sessions.
    /// `reason` is recorded in local close metrics/events, not guaranteed to be sent to the peer.
    /// Stale handles return `InvalidHandle`; worker/socket cleanup is asynchronous. Already
    /// accepted messages can still be in flight, so this is not a remote cancellation barrier.
    pub fn close_session(&self, session: Handle, reason: ErrorCode) -> Result<()> {
        let route = self
            .shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .remove(session)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?;
        self.shared.latest.forget_session(session);
        release_pending_session(&self.shared, &route);
        self.shared.metrics.record_session_closed(reason);
        route.target.request_cleanup(session);
        let mut event = session_event(EventType::SessionClosed, route.endpoint, session);
        event.status = reason;
        publish_lifecycle(&self.shared, event);
        Ok(())
    }

    /// Stops accepting new sends/endpoints, waits up to `drain_timeout` for sessions to disappear,
    /// then closes remaining endpoints and marks the runtime stopped. Zero skips the wait.
    /// This wait is not a flush or peer-delivery guarantee, and shutdown work adds overhead.
    /// Repeated calls after stopping succeed; unrepresentable deadlines return `InvalidArgument`.
    /// Concurrent lifecycle changes may return `InvalidState`. Workers are owned until drop;
    /// queued events remain available to poll, including a best-effort `RuntimeStopped`.
    pub fn stop(&self, drain_timeout: Duration) -> Result<()> {
        if !drain_timeout.is_zero()
            && std::time::Instant::now()
                .checked_add(drain_timeout)
                .is_none()
        {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "drain timeout cannot form a valid deadline",
            ));
        }
        match self.shared.state.load() {
            Lifecycle::Stopped => return self.emit_runtime_stopped(),
            Lifecycle::Running => self.shared.state.begin_draining()?,
            Lifecycle::Draining => {}
            Lifecycle::Created => {
                return Err(RnetError::new(
                    ErrorCode::InvalidState,
                    "runtime was not started",
                ));
            }
        }
        if !drain_timeout.is_zero() {
            let shared = Arc::clone(&self.shared);
            self.runtime.block_on(async move {
                let deadline = tokio::time::Instant::now() + drain_timeout;
                while tokio::time::Instant::now() < deadline {
                    if shared
                        .sessions
                        .lock()
                        .expect("session table poisoned")
                        .handles()
                        .next()
                        .is_none()
                    {
                        break;
                    }
                    sleep(Duration::from_millis(1)).await;
                }
            });
        }
        let endpoints: Vec<_> = self
            .shared
            .endpoints
            .lock()
            .expect("endpoint table poisoned")
            .handles()
            .collect();
        for endpoint in endpoints {
            let _ = self.close_endpoint(endpoint);
        }
        self.shared.state.mark_stopped()?;
        self.emit_runtime_stopped()
    }

    fn emit_runtime_stopped(&self) -> Result<()> {
        let mut emitted = self
            .shared
            .stopped_event_emitted
            .lock()
            .expect("stopped event state poisoned");
        if *emitted {
            return Ok(());
        }
        publish_lifecycle(&self.shared, Event::simple(EventType::RuntimeStopped));
        *emitted = true;
        Ok(())
    }
}

fn close_endpoint_sessions(shared: &Arc<Shared>, endpoint: Handle) {
    let handles: Vec<_> = {
        let sessions = shared.sessions.lock().expect("session table poisoned");
        sessions
            .handles()
            .filter(|handle| {
                sessions
                    .get(*handle)
                    .is_some_and(|route| route.endpoint == endpoint)
            })
            .collect()
    };
    let mut sessions = shared.sessions.lock().expect("session table poisoned");
    for session in handles {
        if let Some(route) = sessions.remove(session) {
            shared.latest.forget_session(session);
            release_pending_session(shared, &route);
            shared.metrics.record_session_closed(ErrorCode::Ok);
            publish_lifecycle(
                shared,
                session_event(EventType::SessionClosed, endpoint, session),
            );
        }
    }
}
