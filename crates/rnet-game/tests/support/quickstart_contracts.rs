// Compiled as a module of the example, not as an independent integration target.
use super::*;

#[test]
fn real_transports_echo_binary_and_empty_payloads_with_bounded_admission() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        for single_slot in [false, true] {
            let options = Options {
                transport,
                accept: true,
                single_slot,
            };
            let report = run(&options, Duration::from_secs(10)).unwrap();
            assert_eq!(
                report.received, 2,
                "{transport:?} single_slot={single_slot}"
            );
            if single_slot {
                assert!(
                    report.blocked > 0,
                    "must exercise real game queue rejection"
                );
            }
        }
    }
}

#[test]
fn rejected_authorization_preserves_category_instead_of_echo_timeout() {
    for transport in [Transport::Tcp, Transport::Udp, Transport::Kcp] {
        let options = Options {
            transport,
            accept: false,
            single_slot: true,
        };
        let error = run(&options, Duration::from_secs(10)).unwrap_err();
        assert_eq!(
            error.code(),
            ErrorCode::AuthRejected,
            "{transport:?}: {error}"
        );
    }
}

#[test]
fn invalid_deadlines_are_rejected_before_setup() {
    let options = Options {
        transport: Transport::Tcp,
        accept: true,
        single_slot: false,
    };
    for timeout in [Duration::ZERO, Duration::MAX] {
        assert_eq!(
            run(&options, timeout).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
    }
}

#[test]
fn cli_rejects_unknown_and_duplicate_arguments() {
    for args in [
        vec!["quic"],
        vec!["tcp", "--unknown"],
        vec!["tcp", "--deny", "--deny"],
        vec!["tcp", "--one-slot", "--one-slot"],
    ] {
        assert_eq!(
            Options::parse(args.into_iter().map(str::to_owned))
                .err()
                .unwrap()
                .code(),
            ErrorCode::InvalidArgument
        );
    }
    let default = Options::parse(std::iter::empty()).unwrap();
    assert_eq!(default.transport, Transport::Kcp);
    let options = Options::parse(
        ["udp", "--one-slot", "--deny"]
            .into_iter()
            .map(str::to_owned),
    )
    .unwrap();
    assert_eq!(options.transport, Transport::Udp);
    assert!(options.single_slot);
    assert!(!options.accept);
}

#[test]
fn failure_events_keep_the_category_instead_of_becoming_timeouts() {
    for (event, expected) in [
        (
            GameEvent::JoinFailed {
                endpoint: 1,
                session: 2,
                reason: ErrorCode::PeerKeyMismatch,
            },
            ErrorCode::PeerKeyMismatch,
        ),
        (
            GameEvent::ProtocolRejected {
                endpoint: 1,
                session: 2,
                reason: ErrorCode::ProtocolError,
            },
            ErrorCode::ProtocolError,
        ),
        (
            GameEvent::ProtocolViolation {
                endpoint: 1,
                session: 2,
            },
            ErrorCode::ProtocolError,
        ),
        (
            GameEvent::EndpointError {
                endpoint: 1,
                status: ErrorCode::IoError,
            },
            ErrorCode::IoError,
        ),
        (
            GameEvent::SessionClosed {
                endpoint: 1,
                session: 2,
                reason: ErrorCode::Ok,
            },
            ErrorCode::InvalidState,
        ),
        (GameEvent::RuntimeStopped, ErrorCode::InvalidState),
    ] {
        assert_eq!(check_event(&event).unwrap_err().code(), expected);
    }
    assert!(check_event(&GameEvent::RuntimeStarted).is_ok());
}

#[test]
fn cleanup_errors_cannot_replace_primary_failure_or_be_reported_as_success() {
    let result = finish(
        Err(RnetError::new(ErrorCode::AuthRejected, "denied")),
        Err(RnetError::new(ErrorCode::WouldBlock, "client stop blocked")),
        Err(RnetError::new(ErrorCode::IoError, "server stop failed")),
    );
    assert_eq!(result.unwrap_err().code(), ErrorCode::AuthRejected);
    let result = finish(
        Ok(Report {
            received: 2,
            blocked: 0,
        }),
        Ok(()),
        Err(RnetError::new(ErrorCode::WouldBlock, "server stop blocked")),
    );
    assert_eq!(result.unwrap_err().code(), ErrorCode::WouldBlock);
}

#[test]
fn partial_admission_does_not_duplicate_the_accepted_prefix() {
    let mut queue = PendingSends::default();
    queue.stage(b"first").unwrap();
    queue.stage(&[]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut accepted = Vec::new();
    assert!(queue
        .flush(
            |payload| {
                if accepted.is_empty() {
                    accepted.push(payload.to_vec());
                    Ok(())
                } else {
                    Err(RnetError::new(ErrorCode::WouldBlock, "full"))
                }
            },
            deadline
        )
        .unwrap());
    assert!(!queue
        .flush(
            |payload| {
                accepted.push(payload.to_vec());
                Ok(())
            },
            deadline
        )
        .unwrap());
    assert_eq!(accepted, vec![b"first".to_vec(), vec![]]);
}

#[test]
fn backpressure_retains_copied_binary_and_empty_payloads_in_order() {
    let mut queue = PendingSends::default();
    let mut original = vec![1, 0, 255];
    queue.stage(&original).unwrap();
    queue.stage(&[]).unwrap();
    original.fill(9);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut calls = 0;
    let blocked = queue.flush(
        |_| {
            calls += 1;
            Err(RnetError::new(ErrorCode::WouldBlock, "bounded admission"))
        },
        deadline,
    );
    assert!(
        matches!(blocked, Ok(true)),
        "must retain work rather than abort: {blocked:?}"
    );
    assert_eq!(calls, 1, "one blocked attempt per iteration, no busy loop");
    let mut received = Vec::new();
    assert!(!queue
        .flush(
            |payload| {
                received.push(payload.to_vec());
                Ok(())
            },
            deadline
        )
        .unwrap());
    assert_eq!(received, vec![vec![1, 0, 255], vec![]]);
}

#[test]
fn expired_retry_never_calls_send_or_discards_work() {
    let mut queue = PendingSends::default();
    queue.stage(b"pending").unwrap();
    let result = queue.flush(|_| panic!("expired work must not be sent"), Instant::now());
    assert_eq!(result.unwrap_err().code(), ErrorCode::Timeout);
    let mut received = Vec::new();
    queue
        .flush(
            |p| {
                received.push(p.to_vec());
                Ok(())
            },
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(received, vec![b"pending".to_vec()]);
}

#[test]
fn terminal_error_is_preserved_without_silent_dequeue() {
    let mut queue = PendingSends::default();
    queue.stage(b"pending").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let error = queue
        .flush(
            |_| Err(RnetError::new(ErrorCode::InvalidHandle, "retired route")),
            deadline,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidHandle);
    assert!(error.to_string().contains("retired route"));
    let mut received = Vec::new();
    queue
        .flush(
            |p| {
                received.push(p.to_vec());
                Ok(())
            },
            deadline,
        )
        .unwrap();
    assert_eq!(received, vec![b"pending".to_vec()]);
}

#[test]
fn count_and_payload_limits_do_not_evict_existing_work() {
    let mut queue = PendingSends::default();
    assert_eq!(
        queue.stage(&[1; 129]).unwrap_err().code(),
        ErrorCode::MessageTooLarge
    );
    queue.stage(&[1; 128]).unwrap();
    queue.stage(&[]).unwrap();
    assert_eq!(
        queue.stage(b"third").unwrap_err().code(),
        ErrorCode::WouldBlock
    );
    let mut received = Vec::new();
    queue
        .flush(
            |p| {
                received.push(p.to_vec());
                Ok(())
            },
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(received, vec![vec![1; 128], vec![]]);
}
