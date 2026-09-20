use std::time::Duration;

use rnet_core::{BufferStore, ErrorCode, Event, EventQueue, EventType, HandleTable, RuntimeState};

#[test]
fn stale_generation_handle_is_rejected() {
    let mut table = HandleTable::new();
    let old = table.insert("first");
    assert_eq!(table.remove(old), Some("first"));

    let replacement = table.insert("second");
    assert_ne!(old, replacement);
    assert!(table.get(old).is_none());
    assert_eq!(table.get(replacement), Some(&"second"));
}

#[test]
fn removing_a_handle_twice_is_harmless() {
    let mut table = HandleTable::new();
    let handle = table.insert(7_u32);

    assert_eq!(table.remove(handle), Some(7));
    assert_eq!(table.remove(handle), None);
}

#[test]
fn bounded_event_queue_reports_full_and_recovers_after_poll() {
    let queue = EventQueue::new(1).expect("positive capacity");
    queue
        .try_push(Event::simple(EventType::RuntimeStarted))
        .unwrap();

    let full = queue
        .try_push(Event::simple(EventType::RuntimeStopped))
        .unwrap_err();
    assert_eq!(full.code(), ErrorCode::WouldBlock);

    let events = queue.poll(2, Duration::ZERO);
    assert_eq!(events.len(), 1);
    queue
        .try_push(Event::simple(EventType::RuntimeStopped))
        .unwrap();
}

#[test]
fn lifecycle_event_replaces_data_when_the_bounded_queue_is_full() {
    let queue = EventQueue::new(2).expect("positive capacity");
    queue.try_push(Event::simple(EventType::Message)).unwrap();
    queue.try_push(Event::simple(EventType::Writable)).unwrap();

    assert!(queue
        .push_priority(Event::simple(EventType::SessionClosed))
        .expect("data notification should be evicted"));

    let events = queue.poll(2, Duration::ZERO);
    assert_eq!(events.len(), 2);
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::SessionClosed));
}

#[test]
fn lifecycle_event_never_replaces_an_older_lifecycle_event() {
    let queue = EventQueue::new(1).expect("positive capacity");
    queue
        .try_push(Event::simple(EventType::SessionOpened))
        .unwrap();

    let rejected = queue
        .push_priority(Event::simple(EventType::SessionClosed))
        .unwrap_err();
    assert_eq!(rejected.code(), ErrorCode::WouldBlock);

    let events = queue.poll(1, Duration::ZERO);
    assert_eq!(events[0].event_type, EventType::SessionOpened);
}

#[test]
fn event_queue_rejects_zero_capacity() {
    assert_eq!(
        EventQueue::new(0).unwrap_err().code(),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn event_queue_enforces_a_total_payload_byte_budget() {
    let queue = EventQueue::new_with_limits(4, 5).expect("valid limits");
    let mut first = Event::simple(EventType::Message);
    first.data = vec![1; 3];
    queue.try_push(first).unwrap();

    let mut overflow = Event::simple(EventType::Message);
    overflow.data = vec![2; 3];
    assert_eq!(
        queue.try_push(overflow).unwrap_err().code(),
        ErrorCode::WouldBlock
    );

    assert_eq!(queue.queued_bytes(), 3);
    queue.poll(1, Duration::ZERO);
    assert_eq!(queue.queued_bytes(), 0);
}

#[test]
fn event_queue_rejects_a_timeout_that_cannot_form_a_deadline() {
    let queue = EventQueue::new(1).expect("positive capacity");

    let error = queue
        .push_timeout(Event::simple(EventType::Message), Duration::MAX)
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert!(queue.is_empty());
}

#[test]
fn runtime_state_transitions_are_idempotent_and_ordered() {
    let state = RuntimeState::created();
    assert!(state.start().is_ok());
    assert!(state.start().is_err());
    assert!(state.begin_draining().is_ok());
    assert!(state.begin_draining().is_ok());
    assert!(state.mark_stopped().is_ok());
    assert!(state.mark_stopped().is_ok());
    assert!(state.begin_draining().is_err());
}

#[test]
fn buffer_tokens_own_bytes_until_exactly_one_release() {
    let store = BufferStore::new();
    let view = store.insert(b"payload".to_vec());

    assert_ne!(view.token, 0);
    assert_eq!(view.len, 7);
    assert_eq!(store.copy(view.token).unwrap(), b"payload");
    assert_eq!(store.outstanding(), 1);
    assert!(store.release(view.token).is_ok());
    assert_eq!(store.outstanding(), 0);
    assert_eq!(
        store.release(view.token).unwrap_err().code(),
        ErrorCode::InvalidHandle
    );
}

#[test]
fn empty_buffer_uses_a_null_pointer_but_still_has_a_release_token() {
    let store = BufferStore::new();
    let view = store.insert(Vec::new());

    assert!(view.ptr.is_null());
    assert_eq!(view.len, 0);
    assert!(store.release(view.token).is_ok());
}
