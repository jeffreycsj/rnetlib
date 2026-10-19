use super::*;

fn event(session: u64, data: &[u8]) -> Event {
    let mut event = Event::simple(EventType::Message);
    event.session = session;
    event.data = data.to_vec();
    event
}

#[test]
fn byte_limited_poll_preserves_fifo_and_exact_boundary_accounting() {
    let queue = EventQueue::new_with_limits(4, 4).unwrap();
    for (id, data) in [
        (1, &b""[..]),
        (2, &b"abc"[..]),
        (3, &b"x"[..]),
        (4, &b""[..]),
    ] {
        queue.try_push(event(id, data)).unwrap();
    }
    let ids = |events: Vec<Event>| events.into_iter().map(|e| e.session).collect::<Vec<_>>();
    assert!(queue
        .poll_with_byte_limit(0, usize::MAX, Duration::ZERO)
        .is_empty());
    assert_eq!(queue.len(), 4);
    assert_eq!(ids(queue.poll_with_byte_limit(4, 0, Duration::ZERO)), [1]);
    // Do not bypass the oversized head to take the small/empty events behind it.
    assert!(queue.poll_with_byte_limit(4, 2, Duration::ZERO).is_empty());
    assert_eq!(queue.queued_bytes(), 4);
    assert_eq!(queue.len(), 3);
    assert_eq!(ids(queue.poll_with_byte_limit(4, 3, Duration::ZERO)), [2]);
    assert_eq!(queue.queued_bytes(), 1);
    assert_eq!(
        ids(queue.poll_with_byte_limit(4, 1, Duration::ZERO)),
        [3, 4]
    );
    assert!(queue.is_empty());
    assert_eq!(queue.queued_bytes(), 0);
}

#[test]
fn rejected_byte_prefix_does_not_release_producer_capacity() {
    let queue = EventQueue::new_with_limits(2, 4).unwrap();
    queue.try_push(event(1, b"1234")).unwrap();
    assert!(queue.poll_with_byte_limit(2, 3, Duration::ZERO).is_empty());
    let producer = queue.clone();
    assert_eq!(
        producer.try_push(event(2, b"x")).unwrap_err().code(),
        ErrorCode::WouldBlock
    );
    let polled = queue.poll_with_byte_limit(usize::MAX, usize::MAX, Duration::ZERO);
    assert_eq!(polled[0].data, b"1234");
    producer.try_push(event(2, b"x")).unwrap();
    assert_eq!(queue.poll(1, Duration::ZERO)[0].data, b"x");
}
