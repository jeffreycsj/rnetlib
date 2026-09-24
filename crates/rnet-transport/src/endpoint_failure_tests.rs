use super::fail_endpoint;
use crate::state::SessionTarget;
use crate::{NetworkRuntime, RuntimeConfig};
use rnet_core::{ErrorCode, EventType, RnetError, Transport};
use std::sync::{Arc, Barrier};
use std::time::Duration;
use tokio::sync::mpsc;

#[test]
fn fatal_endpoint_failure_preserves_other_endpoints_live_send_route() {
    let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
    runtime.poll_events(8, Duration::ZERO);
    let failed = runtime
        .insert_endpoint("127.0.0.1:0".parse().unwrap(), Transport::Udp)
        .unwrap();
    let other = runtime
        .insert_endpoint("127.0.0.1:1".parse().unwrap(), Transport::Tcp)
        .unwrap();
    let (tx, mut receiver) = mpsc::channel(1);
    let session = runtime.insert_session(other, SessionTarget::Tcp(tx));
    fail_endpoint(
        &runtime.shared,
        failed,
        None,
        "datagram_receive",
        RnetError::new(ErrorCode::IoError, "local receive failed"),
    );
    runtime.send_payload(session, b"still live").unwrap();
    assert!(receiver.try_recv().unwrap().bytes.ends_with(b"still live"));
    assert_eq!(runtime.metrics_snapshot().current_sessions, 1);
    assert_eq!(
        runtime
            .metrics_snapshot()
            .closed_sessions(ErrorCode::IoError),
        0
    );
    assert!(runtime.endpoint_local_addr(other).is_ok());
    let events = runtime.poll_events(8, Duration::ZERO);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].endpoint, failed);
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn competing_failures_and_close_have_one_cleanup_winner() {
    for _ in 0..16 {
        let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
        runtime.poll_events(8, Duration::ZERO);
        let endpoint = runtime
            .insert_endpoint("127.0.0.1:0".parse().unwrap(), Transport::Udp)
            .unwrap();
        let (tx, _rx) = mpsc::channel(1);
        let session = runtime.insert_session(endpoint, SessionTarget::Tcp(tx));
        let gate = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for index in 0..8 {
                let runtime = &runtime;
                let gate = gate.clone();
                scope.spawn(move || {
                    gate.wait();
                    if index == 0 {
                        let _ = runtime.close_endpoint(endpoint);
                    } else {
                        fail_endpoint(
                            &runtime.shared,
                            endpoint,
                            None,
                            "datagram_receive",
                            RnetError::new(ErrorCode::IoError, "local receive failed"),
                        );
                    }
                });
            }
        });
        let events = runtime.poll_events(16, Duration::ZERO);
        let closes: Vec<_> = events
            .iter()
            .filter(|event| event.event_type == EventType::SessionClosed)
            .collect();
        assert_eq!(closes.len(), 1);
        assert_eq!(closes[0].session, session);
        let failures = events
            .iter()
            .filter(|event| event.event_type == EventType::EndpointError)
            .count();
        assert_eq!(
            failures,
            usize::from(closes[0].status == ErrorCode::IoError)
        );
        let metrics = runtime.metrics_snapshot();
        assert_eq!(
            metrics.closed_sessions(ErrorCode::Ok) + metrics.closed_sessions(ErrorCode::IoError),
            1
        );
        assert_eq!(metrics.current_endpoints, 0);
        assert_eq!(metrics.current_sessions, 0);
        runtime.stop(Duration::ZERO).unwrap();
    }
}

#[test]
fn pending_client_failure_publishes_join_failed_once_before_close() {
    let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
    runtime.poll_events(8, Duration::ZERO);
    let endpoint = runtime
        .insert_endpoint("127.0.0.1:0".parse().unwrap(), Transport::Kcp)
        .unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let session = runtime
        .insert_pending_session(endpoint, SessionTarget::Tcp(tx), None, true)
        .unwrap();
    for _ in 0..2 {
        fail_endpoint(
            &runtime.shared,
            endpoint,
            Some(session),
            "datagram_receive",
            RnetError::new(ErrorCode::IoError, "local receive failed"),
        );
    }
    let events = runtime.poll_events(8, Duration::ZERO);
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        [
            EventType::EndpointError,
            EventType::JoinFailed,
            EventType::SessionClosed
        ]
    );
    assert!(events
        .iter()
        .all(|event| event.status == ErrorCode::IoError));
    assert_eq!(events[1].session, session);
    assert_eq!(events[2].session, session);
    assert_eq!(runtime.metrics_snapshot().pending_handshakes, 0);
    assert_eq!(
        runtime
            .metrics_snapshot()
            .closed_sessions(ErrorCode::IoError),
        1
    );
    runtime.stop(Duration::ZERO).unwrap();
}

#[test]
fn empty_endpoint_failure_releases_capacity_and_explicit_close_suppresses_late_errors() {
    let runtime = NetworkRuntime::new(RuntimeConfig::production()).unwrap();
    runtime.poll_events(8, Duration::ZERO);
    for explicitly_closed in [false, true] {
        let endpoint = runtime
            .insert_endpoint("127.0.0.1:0".parse().unwrap(), Transport::Udp)
            .unwrap();
        if explicitly_closed {
            runtime.close_endpoint(endpoint).unwrap();
        }
        fail_endpoint(
            &runtime.shared,
            endpoint,
            None,
            "datagram_receive",
            RnetError::new(ErrorCode::IoError, "local receive failed"),
        );
        let events = runtime.poll_events(8, Duration::ZERO);
        assert_eq!(events.len(), usize::from(!explicitly_closed));
        assert_eq!(runtime.metrics_snapshot().current_endpoints, 0);
        assert_eq!(runtime.metrics_snapshot().current_sessions, 0);
    }
    runtime.stop(Duration::ZERO).unwrap();
}
