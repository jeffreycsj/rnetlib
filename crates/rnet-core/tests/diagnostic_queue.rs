use rnet_core::{ErrorCode, Event, EventQueue, EventType};
use std::time::Duration;

#[test]
fn optional_failure_details_never_cost_lifecycle_delivery_or_extra_data_evictions() {
    for failure in [EventType::SessionClosed, EventType::JoinFailed] {
        for existing in [EventType::AuthRequest, EventType::Message] {
            let queue = EventQueue::new_with_limits(2, 4).unwrap();
            let mut prior = Event::simple(existing);
            prior.data = b"keep".to_vec();
            queue.try_push(prior).unwrap();
            let mut closed = Event::simple(failure);
            closed.status = ErrorCode::IoError;
            closed.data = b"phase=tcp_read peer closed".to_vec();
            assert!(!queue
                .push_priority(closed)
                .expect("details must not block the transition"));
            let events = queue.poll(2, Duration::ZERO);
            assert_eq!(events.len(), 2);
            assert_eq!(events[0].event_type, existing);
            assert_eq!(events[0].data, b"keep");
            assert_eq!(events[1].event_type, failure);
            assert_eq!(events[1].status, ErrorCode::IoError);
            assert!(events[1].data.is_empty());
            assert_eq!(queue.queued_bytes(), 0);
        }
    }
}

#[test]
fn optional_failure_details_are_preserved_when_the_budget_allows_them() {
    let queue = EventQueue::new_with_limits(1, 4).unwrap();
    let mut event = Event::simple(EventType::SessionClosed);
    event.data = b"info".to_vec();
    assert!(!queue.push_priority(event).unwrap());
    assert_eq!(queue.queued_bytes(), 4);
    assert_eq!(queue.poll(1, Duration::ZERO)[0].data, b"info");
    assert_eq!(queue.queued_bytes(), 0);
}

#[test]
fn authentication_bytes_are_never_treated_as_discardable_diagnostics() {
    let queue = EventQueue::new_with_limits(1, 4).unwrap();
    let mut auth = Event::simple(EventType::AuthRequest);
    auth.data = b"credential".to_vec();
    assert_eq!(
        queue.push_priority(auth).unwrap_err().code(),
        ErrorCode::WouldBlock
    );
    assert!(queue.poll(1, Duration::ZERO).is_empty());
}
