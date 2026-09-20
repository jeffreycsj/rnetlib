use super::realtime::{LatestQueue, QueueEffect};
use bytes::Bytes;
use rnet_core::ErrorCode;

#[test]
fn latest_queue_replaces_by_session_and_key_without_reordering_other_keys() {
    let mut queue = LatestQueue::new(100, 100, 2).unwrap();
    assert_eq!(
        queue
            .enqueue(1, 7, Bytes::from_static(b"old"), None)
            .unwrap(),
        QueueEffect::Queued
    );
    assert_eq!(
        queue
            .enqueue(1, 8, Bytes::from_static(b"other"), None)
            .unwrap(),
        QueueEffect::Queued
    );
    assert_eq!(
        queue
            .enqueue(1, 7, Bytes::from_static(b"newer"), Some(42))
            .unwrap(),
        QueueEffect::Replaced
    );
    assert_eq!(queue.snapshot().queued_messages, 2);
    assert_eq!(queue.snapshot().queued_bytes, 42);
    assert_eq!(queue.snapshot().replaced, 1);
    let first = queue.pop_next().unwrap();
    assert_eq!(
        (first.session, first.key, first.payload.as_ref(), first.tick),
        (1, 7, b"newer".as_slice(), Some(42))
    );
    assert_eq!(queue.pop_next().unwrap().key, 8);
    assert!(queue.pop_next().is_none());
}

#[test]
fn latest_queue_enforces_key_and_byte_limits_without_losing_existing_value() {
    let mut queue = LatestQueue::new(30, 21, 1).unwrap();
    queue
        .enqueue(1, 7, Bytes::from_static(b"first"), None)
        .unwrap();
    assert_eq!(
        queue
            .enqueue(1, 8, Bytes::from_static(b"x"), None)
            .unwrap_err()
            .code(),
        ErrorCode::WouldBlock
    );
    assert_eq!(
        queue
            .enqueue(1, 7, Bytes::from_static(b"toolong"), None)
            .unwrap_err()
            .code(),
        ErrorCode::WouldBlock
    );
    assert_eq!(queue.pop_next().unwrap().payload.as_ref(), b"first");
    assert_eq!(queue.snapshot().queued_bytes, 0);
    assert_eq!(queue.snapshot().admission_rejected, 2);
}

#[test]
fn latest_queue_rotates_backpressured_sessions_and_cleans_closed_sessions() {
    let mut queue = LatestQueue::new(100, 100, 2).unwrap();
    queue.enqueue(1, 1, Bytes::from_static(b"a"), None).unwrap();
    queue.enqueue(2, 1, Bytes::from_static(b"b"), None).unwrap();
    let blocked = queue.pop_next().unwrap();
    queue.requeue(blocked);
    assert_eq!(queue.pop_next().unwrap().session, 2);
    assert_eq!(queue.forget_session(1), 1);
    assert_eq!(queue.snapshot().closed_dropped, 1);
    assert!(queue.pop_next().is_none());
}

#[test]
fn newer_value_wins_if_a_blocked_send_returns_after_replacement() {
    let mut queue = LatestQueue::new(100, 100, 2).unwrap();
    queue
        .enqueue(1, 9, Bytes::from_static(b"old"), None)
        .unwrap();
    let blocked = queue.pop_next().unwrap();
    queue
        .enqueue(1, 9, Bytes::from_static(b"new"), None)
        .unwrap();
    queue.requeue(blocked);
    assert_eq!(queue.pop_next().unwrap().payload.as_ref(), b"new");
    assert_eq!(queue.snapshot().replaced, 1);
}

#[test]
fn empty_messages_and_high_churn_cannot_grow_the_pending_key_count() {
    let mut queue = LatestQueue::new(32, 32, 10).unwrap();
    for _ in 0..10_000 {
        queue.enqueue(1, 1, Bytes::new(), Some(0)).unwrap();
    }
    queue.enqueue(1, 2, Bytes::new(), None).unwrap();
    assert_eq!(
        queue.enqueue(1, 3, Bytes::new(), None).unwrap_err().code(),
        ErrorCode::WouldBlock
    );
    assert_eq!(queue.snapshot().queued_messages, 2);
    assert_eq!(queue.snapshot().queued_bytes, 32);
    assert_eq!(queue.snapshot().replaced, 9_999);
}
