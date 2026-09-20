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

    pub fn close_session(&self, session: Handle, reason: ErrorCode) -> Result<()> {
        let route = self
            .shared
            .sessions
            .lock()
            .expect("session table poisoned")
            .remove(session)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid session"))?;
        release_pending_session(&self.shared, &route);
        self.shared.metrics.record_session_closed(reason);
        route.target.request_cleanup(session);
        let mut event = session_event(EventType::SessionClosed, route.endpoint, session);
        event.status = reason;
        publish_lifecycle(&self.shared, event);
        Ok(())
    }

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
            release_pending_session(shared, &route);
            shared.metrics.record_session_closed(ErrorCode::Ok);
            publish_lifecycle(
                shared,
                session_event(EventType::SessionClosed, endpoint, session),
            );
        }
    }
}
