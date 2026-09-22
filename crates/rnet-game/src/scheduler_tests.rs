use crate::scheduler::{GamePriority, ScheduledMessage, ScheduledQueue, ScheduledQueueConfig};
use bytes::Bytes;
use std::time::{Duration, Instant};

fn message(session: u64, priority: GamePriority, payload: &[u8]) -> ScheduledMessage {
    ScheduledMessage {
        session,
        payload: Bytes::copy_from_slice(payload),
        sequence: None,
        tick: None,
        correlation_id: 0,
        priority,
        enqueued_at: Instant::now(),
        expires_at: None,
    }
}

#[test]
fn weighted_scheduler_prioritizes_latency_without_starving_low_priority() {
    let mut queue = ScheduledQueue::from_config(ScheduledQueueConfig::default()).unwrap();
    for index in 0..30 {
        queue
            .enqueue(message(1, GamePriority::Critical, &[index]))
            .unwrap();
    }
    queue
        .enqueue(message(2, GamePriority::Low, b"low"))
        .unwrap();

    let first_cycle: Vec<_> = (0..15)
        .map(|_| queue.pop_next(Instant::now()).unwrap())
        .collect();
    assert!(first_cycle
        .iter()
        .any(|message| message.payload == b"low"[..]));
    assert!(
        first_cycle
            .iter()
            .filter(|message| message.priority == GamePriority::Critical)
            .count()
            > first_cycle
                .iter()
                .filter(|message| message.priority == GamePriority::Low)
                .count()
    );
}

#[test]
fn expired_messages_release_budget_and_are_counted() {
    let mut queue = ScheduledQueue::from_config(ScheduledQueueConfig {
        max_queued_bytes: 64,
        max_session_queued_bytes: 64,
        max_queued_messages: 4,
        flush_batch: 4,
    })
    .unwrap();
    let mut expired = message(1, GamePriority::High, b"expired");
    expired.expires_at = Some(Instant::now() - Duration::from_millis(1));
    queue.enqueue(expired).unwrap();

    assert!(queue.pop_next(Instant::now()).is_none());
    let snapshot = queue.snapshot();
    assert_eq!(snapshot.expired_dropped, 1);
    assert_eq!(snapshot.queued_messages, 0);
    assert_eq!(snapshot.queued_bytes, 0);
}

#[test]
fn closing_one_session_drops_only_its_scheduled_messages() {
    let mut queue = ScheduledQueue::from_config(ScheduledQueueConfig::default()).unwrap();
    queue
        .enqueue(message(10, GamePriority::Normal, b"closed"))
        .unwrap();
    queue
        .enqueue(message(11, GamePriority::Normal, b"alive"))
        .unwrap();

    assert_eq!(queue.forget_session(10), 1);
    assert_eq!(queue.snapshot().closed_dropped, 1);
    let message = queue.pop_next(Instant::now()).unwrap();
    assert_eq!(message.session, 11);
    queue.record_forwarded(&message);
}

#[test]
fn backpressure_rotation_keeps_the_original_budget_reservation() {
    let mut queue = ScheduledQueue::from_config(ScheduledQueueConfig::default()).unwrap();
    queue
        .enqueue(message(1, GamePriority::High, b"reserved"))
        .unwrap();
    let before = queue.snapshot();
    let pending = queue.pop_next(Instant::now()).unwrap();
    assert_eq!(queue.snapshot(), before);
    queue.requeue(pending);
    let requeued = queue.snapshot();
    assert_eq!(requeued.queued_messages, before.queued_messages);
    assert_eq!(requeued.queued_bytes, before.queued_bytes);
    assert_eq!(requeued.backpressure_requeued, 1);

    let pending = queue.pop_next(Instant::now()).unwrap();
    queue.record_forwarded(&pending);
    let snapshot = queue.snapshot();
    assert_eq!(snapshot.queued_messages, 0);
    assert_eq!(snapshot.queued_bytes, 0);
    assert_eq!(snapshot.admitted_by_priority[GamePriority::High.index()], 1);
    assert_eq!(
        snapshot.forwarded_by_priority[GamePriority::High.index()],
        1
    );
    assert_eq!(snapshot.queue_delay.sample_count, 1);
}

#[test]
fn tick_queue_delay_counts_only_tick_annotated_messages() {
    let mut queue = ScheduledQueue::from_config(ScheduledQueueConfig::default()).unwrap();
    let plain = message(1, GamePriority::Normal, b"plain");
    let mut ticked = message(1, GamePriority::Normal, b"tick");
    ticked.tick = Some(u32::MAX);
    queue.enqueue(plain).unwrap();
    queue.enqueue(ticked).unwrap();
    for _ in 0..2 {
        let pending = queue.pop_next(Instant::now()).unwrap();
        queue.record_forwarded(&pending);
    }

    let snapshot = queue.snapshot();
    assert_eq!(snapshot.queue_delay.sample_count, 2);
    assert_eq!(snapshot.tick_queue_delay.sample_count, 1);
}

#[test]
fn one_busy_session_cannot_starve_another_at_the_same_priority() {
    let mut queue = ScheduledQueue::from_config(ScheduledQueueConfig::default()).unwrap();
    for index in 0..100 {
        queue
            .enqueue(message(1, GamePriority::Normal, &[index]))
            .unwrap();
    }
    queue
        .enqueue(message(2, GamePriority::Normal, b"other-player"))
        .unwrap();

    let first = queue.pop_next(Instant::now()).unwrap();
    let second = queue.pop_next(Instant::now()).unwrap();

    assert_eq!(first.session, 1);
    assert_eq!(second.session, 2);
}
