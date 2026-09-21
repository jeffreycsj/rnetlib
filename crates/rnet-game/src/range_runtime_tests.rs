use super::*;
use crate::config::{GameProtocolRange, GameRangeServerConfig, GameRuntimeConfig};
use crate::range_state::RangeRuntimeState;
use bytes::Bytes;
use rnet_core::EventType;
use std::time::Duration;

fn range_state() -> RangeRuntimeState {
    RangeRuntimeState::with_buffer_limits(4_096, 64 * 1024 * 1024)
}

#[test]
fn late_duplicate_select_does_not_disconnect_a_ready_udp_client() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    let now = Instant::now();
    let session = RangeSession {
        endpoint: 31,
        protocol: GameProtocolRange::new(55, 2, 5),
        nonce: [7; 16],
        selected: Some(4),
        server_handle: 99,
        old_session: None,
        server_resume: false,
        phase: Phase::ClientReady,
        deadline: now + Duration::from_secs(1),
        next_retry: now,
        buffered: VecDeque::new(),
        buffered_bytes: 0,
    };
    let control = session.control(SELECT);
    runtime.range.lock().unwrap().sessions.insert(32, session);
    let mut event = Event::simple(EventType::GameControl);
    event.endpoint = 31;
    event.session = 32;
    assert!(runtime
        .handle_range_control(&event, &control)
        .unwrap()
        .is_none());
}

#[test]
fn completed_early_messages_respect_public_poll_capacity() {
    let mut state = range_state();
    for byte in [1, 2] {
        state.push_completed_for_test(GameEvent::Message(GameMessage {
            endpoint: 1,
            session: 2,
            sequence: None,
            tick: None,
            payload: Bytes::from(vec![byte]),
        }));
    }
    let mut first = Vec::new();
    state.drain_completed(&mut first, 1);
    assert_eq!(first.len(), 1);
    let mut second = Vec::new();
    state.drain_completed(&mut second, 1);
    assert_eq!(second.len(), 1);
}

#[test]
fn resumed_mapping_precedes_buffered_business_data() {
    let mut state = range_state();
    state.push_completed_for_test(GameEvent::Message(GameMessage {
        endpoint: 1,
        session: 3,
        sequence: None,
        tick: None,
        payload: Bytes::from_static(b"early"),
    }));
    let mut output = Vec::new();
    state.queue_public(
        GameEvent::SessionResumed {
            endpoint: 1,
            old_session: 2,
            new_session: 3,
        },
        &mut output,
        1,
    );
    assert!(matches!(
        output.as_slice(),
        [GameEvent::SessionResumed { new_session: 3, .. }]
    ));
    let mut next = Vec::new();
    state.drain_completed(&mut next, 1);
    assert!(matches!(next.as_slice(), [GameEvent::Message(message)] if message.session == 3));
}

#[test]
fn closing_range_session_discards_deferred_ready_and_message_events() {
    let mut state = range_state();
    state.push_completed_for_test(GameEvent::SessionReady {
        endpoint: 1,
        session: 9,
    });
    state.push_completed_for_test(GameEvent::Message(GameMessage {
        endpoint: 1,
        session: 9,
        sequence: None,
        tick: None,
        payload: Bytes::from_static(b"late"),
    }));
    state.forget_session(9);
    let mut output = Vec::new();
    state.drain_completed(&mut output, 10);
    assert!(
        output.is_empty(),
        "closed sessions cannot publish stale deferred events"
    );
}

#[test]
fn stopping_runtime_clears_v4_listener_policy() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    let listener = runtime
        .listen_range(GameRangeServerConfig {
            transport: rnet_core::Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: rnet_security::Keypair::generate().unwrap(),
            initial_encryption: true,
            protocol: GameProtocolRange::new(3, 1, 2),
        })
        .unwrap();
    assert!(runtime
        .range
        .lock()
        .unwrap()
        .server_endpoints
        .contains_key(&listener));
    runtime.stop(Duration::ZERO).unwrap();
    assert!(runtime.range.lock().unwrap().server_endpoints.is_empty());
}

#[test]
fn failed_join_does_not_remove_v4_listener_policy() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    let listener = runtime
        .listen_range(GameRangeServerConfig {
            transport: rnet_core::Transport::Tcp,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            local_key: rnet_security::Keypair::generate().unwrap(),
            initial_encryption: true,
            protocol: GameProtocolRange::new(4, 1, 3),
        })
        .unwrap();
    let mut failed = Event::simple(EventType::JoinFailed);
    failed.endpoint = listener;
    failed.session = 123;
    runtime.convert_event(failed).unwrap();
    assert!(runtime
        .range
        .lock()
        .unwrap()
        .server_endpoints
        .contains_key(&listener));
}
