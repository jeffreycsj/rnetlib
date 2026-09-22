use crate::config::{GameProtocol, GameRuntimeConfig};
use crate::event::GameEvent;
use crate::resume::{ResumeRegistry, ResumeScope};
use crate::runtime::GameRuntime;
use rnet_core::ErrorCode;
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

fn protocol() -> GameProtocol {
    GameProtocol::new(37, 2)
}

fn scope(session: u64, peer_key: [u8; 32]) -> ResumeScope {
    ResumeScope {
        endpoint: 10,
        session,
        peer_key,
        protocol: protocol(),
    }
}

#[test]
fn ticket_is_bound_to_peer_and_protocol_and_can_be_consumed_once() {
    let mut tickets = ResumeRegistry::new(4).unwrap();
    let now = Instant::now();
    let peer = [7; 32];
    let ticket = tickets
        .issue(scope(111, peer), b"player-7", now, Duration::from_secs(30))
        .unwrap();
    assert_eq!(ticket.len(), 48);
    assert!(tickets
        .consume(&ticket, 10, [8; 32], protocol(), now)
        .is_err());
    assert!(tickets
        .consume(&ticket, 10, peer, GameProtocol::new(37, 3), now)
        .is_err());
    let claim = tickets.consume(&ticket, 10, peer, protocol(), now).unwrap();
    assert_eq!(claim.old_session, 111);
    assert_eq!(claim.identity, b"player-7");
    assert_eq!(
        tickets
            .consume(&ticket, 10, peer, protocol(), now)
            .err()
            .unwrap()
            .code(),
        ErrorCode::ProtocolError
    );
}

#[test]
fn expired_and_reissued_tickets_cannot_reclaim_a_session() {
    let mut tickets = ResumeRegistry::new(1).unwrap();
    let now = Instant::now();
    let peer = [1; 32];
    let first = tickets
        .issue(scope(111, peer), b"a", now, Duration::from_secs(3))
        .unwrap();
    let second = tickets
        .issue(scope(111, peer), b"b", now, Duration::from_secs(3))
        .unwrap();
    assert!(tickets.consume(&first, 10, peer, protocol(), now).is_err());
    assert!(tickets
        .consume(&second, 10, peer, protocol(), now + Duration::from_secs(3))
        .is_err());
    tickets
        .issue(
            scope(222, peer),
            b"c",
            now + Duration::from_secs(3),
            Duration::from_secs(3),
        )
        .unwrap();
}

#[test]
fn revoked_tickets_and_invalid_sizes_are_rejected() {
    let mut tickets = ResumeRegistry::new(1).unwrap();
    let now = Instant::now();
    let peer = [1; 32];
    let ticket = tickets
        .issue(scope(111, peer), b"a", now, Duration::from_secs(3))
        .unwrap();
    tickets.revoke_session(111);
    assert!(tickets.consume(&ticket, 10, peer, protocol(), now).is_err());
    assert!(tickets
        .consume(&ticket[..47], 10, peer, protocol(), now)
        .is_err());
    assert!(tickets
        .issue(scope(222, peer), &[0; 65], now, Duration::from_secs(3))
        .is_err());
}

#[test]
fn racing_claimants_cannot_both_consume_one_ticket() {
    let now = Instant::now();
    let peer = [4; 32];
    let mut tickets = ResumeRegistry::new(1).unwrap();
    let ticket = tickets
        .issue(scope(111, peer), b"one", now, Duration::from_secs(30))
        .unwrap();
    let registry = Arc::new(Mutex::new(tickets));
    let start = Arc::new(Barrier::new(9));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let start = Arc::clone(&start);
            let ticket = ticket.clone();
            std::thread::spawn(move || {
                start.wait();
                registry
                    .lock()
                    .unwrap()
                    .consume(&ticket, 10, peer, protocol(), now)
                    .is_ok()
            })
        })
        .collect();
    start.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|consumed| *consumed)
            .count(),
        1
    );
}

#[test]
fn resume_ticket_storm_stays_bounded_and_recovers_capacity() {
    const CAPACITY: usize = 1024;
    let now = Instant::now();
    let peer = [9; 32];
    let mut tickets = ResumeRegistry::new(CAPACITY).unwrap();
    let issued: Vec<_> = (1..=CAPACITY as u64)
        .map(|session| {
            tickets
                .issue(
                    scope(session, peer),
                    &session.to_be_bytes(),
                    now,
                    Duration::from_secs(30),
                )
                .unwrap()
        })
        .collect();
    assert_eq!(tickets.outstanding(), CAPACITY);
    assert_eq!(
        tickets
            .issue(
                scope(CAPACITY as u64 + 1, peer),
                b"overflow",
                now,
                Duration::from_secs(30),
            )
            .unwrap_err()
            .code(),
        ErrorCode::WouldBlock
    );

    for ticket in issued.iter().rev() {
        tickets
            .consume(ticket, 10, peer, protocol(), now)
            .expect("every unique storm ticket remains consumable");
    }
    assert_eq!(tickets.outstanding(), 0);
    tickets
        .issue(
            scope(CAPACITY as u64 + 1, peer),
            b"recovered",
            now,
            Duration::from_secs(30),
        )
        .expect("capacity recovers after consumption");
}

#[test]
fn ticket_cannot_move_to_another_listener_and_listener_close_revokes_it() {
    let mut tickets = ResumeRegistry::new(2).unwrap();
    let now = Instant::now();
    let peer = [3; 32];
    let ticket = tickets
        .issue(scope(111, peer), b"player", now, Duration::from_secs(30))
        .unwrap();
    assert!(tickets.consume(&ticket, 11, peer, protocol(), now).is_err());
    tickets.revoke_endpoint(10);
    assert!(tickets.consume(&ticket, 10, peer, protocol(), now).is_err());
}

#[test]
fn debug_output_does_not_expose_join_credentials() {
    let event = GameEvent::AuthRequest {
        endpoint: 1,
        session: 2,
        client_public_key: [0; 32],
        join_ticket: b"secret-join-proof".to_vec().into(),
        build_id: 0,
        capabilities: 0,
    };
    let credential_debug = format!("{:?}", b"secret-join-proof".to_vec());
    assert!(!format!("{event:?}").contains(&credential_debug));
    let resume = GameEvent::ResumeRequest {
        endpoint: 1,
        session: 3,
        old_session: 2,
        identity: b"private-player-id".to_vec().into(),
        client_public_key: [0; 32],
        join_ticket: b"secret-join-proof".to_vec().into(),
        build_id: 0,
        capabilities: 0,
    };
    let identity_debug = format!("{:?}", b"private-player-id".to_vec());
    let debug = format!("{resume:?}");
    assert!(!debug.contains(&credential_debug));
    assert!(!debug.contains(&identity_debug));
}

#[test]
fn abandoned_resume_mapping_is_removed_when_new_session_closes() {
    let runtime = GameRuntime::new(GameRuntimeConfig::production()).unwrap();
    {
        let mut state = runtime.resume.lock().unwrap();
        state.inflight_server.insert(10, 20);
        state.revoked_inflight.insert(10);
    }
    runtime.forget_resume_session(20);
    let state = runtime.resume.lock().unwrap();
    assert!(state.inflight_server.is_empty());
    assert!(state.revoked_inflight.is_empty());
}
