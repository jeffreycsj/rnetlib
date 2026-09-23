//! Single-winner session cleanup and bounded, local-only close diagnostics.

use crate::state::{publish_lifecycle, release_pending_session, session_event, Shared};
use rnet_core::{ErrorCode, EventType, Handle, RnetError};
use std::sync::Arc;

pub(crate) fn remove_session(shared: &Arc<Shared>, endpoint: Handle, session: Handle) {
    remove_session_with_reason(shared, endpoint, session, ErrorCode::Ok);
}

pub(crate) fn remove_session_with_reason(
    shared: &Arc<Shared>,
    _endpoint: Handle,
    session: Handle,
    reason: ErrorCode,
) {
    close(shared, session, reason, None);
}

/// Errors must be generated locally by the library, never copied from peer/application payloads.
/// The event retains the original status; its optional data is diagnostic text, not a wire field.
pub(crate) fn remove_session_with_error(
    shared: &Arc<Shared>,
    session: Handle,
    phase: &'static str,
    error: RnetError,
) {
    close(shared, session, error.code(), Some((phase, error)));
}

fn close(
    shared: &Arc<Shared>,
    session: Handle,
    reason: ErrorCode,
    detail: Option<(&'static str, RnetError)>,
) {
    if let Some(route) = shared
        .sessions
        .lock()
        .expect("session table poisoned")
        .remove(session)
    {
        // Removing the route chooses the winner: concurrent cleanup cannot duplicate either the
        // event or counters. Keep the same cleanup/notification order for callers without detail.
        shared.latest.forget_session(session);
        release_pending_session(shared, &route);
        shared.metrics.record_session_closed(reason);
        route.target.request_cleanup(session);
        let mut event = session_event(EventType::SessionClosed, route.endpoint, session);
        event.status = reason;
        if let Some((phase, error)) = detail {
            event.data = diagnostic_bytes(phase, &error);
        }
        publish_lifecycle(shared, event);
    }
}

fn diagnostic_bytes(phase: &str, error: &RnetError) -> Vec<u8> {
    let mut text = format!("phase={phase} {error}");
    let mut limit = text.len().min(1024);
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    text.truncate(limit);
    // Vec/String truncation alone retains the original allocation outside queue byte accounting.
    text.into_bytes().into_boxed_slice().into_vec()
}

#[cfg(test)]
mod tests {
    use super::{diagnostic_bytes, remove_session_with_error, remove_session_with_reason};
    use crate::state::SessionTarget;
    use crate::{NetworkRuntime, RuntimeConfig};
    use rnet_core::{ErrorCode, EventType, RnetError};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[test]
    fn racing_failures_publish_and_count_one_close() {
        let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
        runtime.poll_events(8, Duration::ZERO);
        let (tx, _rx) = mpsc::channel(1);
        let session = runtime.insert_session(77, SessionTarget::Tcp(tx));
        let gate = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let shared = runtime.shared.clone();
                let gate = gate.clone();
                scope.spawn(move || {
                    gate.wait();
                    remove_session_with_error(
                        &shared,
                        session,
                        "tcp_read",
                        RnetError::new(ErrorCode::IoError, "test failure"),
                    );
                });
            }
        });
        remove_session_with_reason(&runtime.shared, 77, session, ErrorCode::Cancelled);
        let events = runtime.poll_events(8, Duration::ZERO);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::SessionClosed);
        assert_eq!(events[0].session, session);
        assert_eq!(events[0].endpoint, 77);
        assert!(std::str::from_utf8(&events[0].data)
            .unwrap()
            .contains("test failure"));
        let metrics = runtime.metrics_snapshot();
        assert_eq!(metrics.closed_sessions(ErrorCode::IoError), 1);
        assert_eq!(metrics.closed_sessions(ErrorCode::Cancelled), 0);
        assert_eq!(metrics.current_sessions, 0);
        runtime.stop(Duration::ZERO).unwrap();
    }

    #[test]
    fn close_details_are_bounded_utf8_with_the_phase_preserved() {
        for message in [String::new(), "x".repeat(4096), "中".repeat(4096)] {
            let error = RnetError::new(ErrorCode::IoError, message);
            let bytes = diagnostic_bytes("tcp_read", &error);
            assert!(bytes.len() <= 1024);
            assert!(
                bytes.capacity() <= 1024,
                "truncation retained an oversized allocation"
            );
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.starts_with("phase=tcp_read IoError:"));
        }
    }
}
