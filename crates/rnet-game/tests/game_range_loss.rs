use rnet_core::Transport;
use rnet_game::{
    GameEvent, GameProtocolRange, GameRangeClientConfig, GameRangeServerConfig, GameRuntime,
    GameRuntimeConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

struct LossyUdpProxy {
    address: SocketAddr,
    client_to_server_drop_countdown: Arc<AtomicUsize>,
    server_to_client_drop_countdown: Arc<AtomicUsize>,
    server_to_client_forward_budget: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl LossyUdpProxy {
    fn start(server: SocketAddr) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind UDP proxy");
        socket
            .set_read_timeout(Some(Duration::from_millis(10)))
            .expect("set proxy timeout");
        let address = socket.local_addr().expect("proxy address");
        let drop_client_to_server = Arc::new(AtomicUsize::new(usize::MAX));
        let drop_server_to_client = Arc::new(AtomicUsize::new(usize::MAX));
        let server_to_client_forward_budget = Arc::new(AtomicUsize::new(usize::MAX));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let client_drops = Arc::clone(&drop_client_to_server);
            let server_drops = Arc::clone(&drop_server_to_client);
            let server_forward_budget = Arc::clone(&server_to_client_forward_budget);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut client = None;
                let mut buffer = vec![0; 65_535];
                while !stop.load(Ordering::Acquire) {
                    let (length, source) = match socket.recv_from(&mut buffer) {
                        Ok(received) => received,
                        Err(error)
                            if matches!(
                                error.kind(),
                                ErrorKind::WouldBlock | ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    };
                    if source == server {
                        if consume_drop(&server_drops) {
                            continue;
                        }
                        if !consume_forward_budget(&server_forward_budget) {
                            continue;
                        }
                        if let Some(client) = client {
                            let _ = socket.send_to(&buffer[..length], client);
                        }
                    } else {
                        client = Some(source);
                        if consume_drop(&client_drops) {
                            continue;
                        }
                        let _ = socket.send_to(&buffer[..length], server);
                    }
                }
            })
        };
        Self {
            address,
            client_to_server_drop_countdown: drop_client_to_server,
            server_to_client_drop_countdown: drop_server_to_client,
            server_to_client_forward_budget,
            stop,
            worker: Some(worker),
        }
    }

    fn drop_second_server_packet(&self) {
        // The first server packet after authorization completes transport authentication.
        // The following packet carries wire-v4 SELECT, whose retransmission is under test.
        self.server_to_client_drop_countdown
            .store(1, Ordering::Release);
    }

    fn assert_armed_drops_were_observed(&self) {
        assert_eq!(
            self.server_to_client_drop_countdown.load(Ordering::Acquire),
            usize::MAX
        );
    }

    fn forward_one_server_packet_then_block(&self) {
        // AuthDecision opens the protected transport. Blocking subsequent server records keeps
        // wire-v4 below READY while the original direct connection remains unaffected.
        self.server_to_client_forward_budget
            .store(1, Ordering::Release);
    }
}

impl Drop for LossyUdpProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake recv_from so teardown does not depend on the read timeout.
        if let Ok(socket) = UdpSocket::bind("127.0.0.1:0") {
            let _ = socket.send_to(&[0], self.address);
        }
        if let Some(worker) = self.worker.take() {
            worker.join().expect("UDP proxy thread");
        }
    }
}

fn consume_drop(counter: &AtomicUsize) -> bool {
    matches!(
        counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
            if remaining == usize::MAX {
                None
            } else if remaining == 0 {
                Some(usize::MAX)
            } else {
                Some(remaining - 1)
            }
        }),
        Ok(0)
    )
}

fn consume_forward_budget(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
            if remaining == usize::MAX || remaining == 0 {
                None
            } else {
                Some(remaining - 1)
            }
        })
        .map_or_else(
            |remaining| remaining == usize::MAX,
            |remaining| remaining > 0,
        )
}

#[test]
fn wire_v4_udp_and_kcp_recover_when_negotiation_packets_are_lost() {
    for transport in [Transport::Udp, Transport::Kcp] {
        run_loss_case(transport);
    }
}

#[test]
fn failed_v4_resume_keeps_the_old_session_usable() {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let server_public = server_key.public.clone();
    let server = GameRuntime::new(GameRuntimeConfig::production()).expect("server runtime");
    let client = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production(),
        ClientSecurity::pinned(client_key, server_public),
    )
    .expect("client runtime");
    let listener = server
        .listen_range(GameRangeServerConfig {
            transport: Transport::Udp,
            bind_addr: "127.0.0.1:0".parse().expect("bind address"),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocolRange::new(0x5253_554d, 1, 3),
        })
        .expect("range listener");
    let listener_address = server
        .endpoint_local_addr(listener)
        .expect("listener address");
    let old_endpoint = client
        .connect_range(GameRangeClientConfig {
            transport: Transport::Udp,
            bind_addr: None,
            remote_addr: listener_address,
            join_ticket: b"old-login".to_vec(),
            protocol: GameProtocolRange::new(0x5253_554d, 1, 3),
        })
        .expect("old connection");

    let deadline = Instant::now() + Duration::from_secs(4);
    let mut old_server = None;
    let mut old_client = None;
    while (old_server.is_none() || old_client.is_none()) && Instant::now() < deadline {
        for event in server.poll(32, Duration::from_millis(5)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    server.auth_decide(session, true).expect("authorize old")
                }
                GameEvent::SessionReady { session, .. } => old_server = Some(session),
                _ => {}
            }
        }
        for event in client.poll(32, Duration::from_millis(5)) {
            if let GameEvent::SessionReady { endpoint, session } = event {
                if endpoint == old_endpoint {
                    old_client = Some(session);
                }
            }
        }
    }
    let old_server = old_server.expect("old server ready");
    let old_client = old_client.expect("old client ready");
    server
        .issue_resume_ticket(old_server, b"player")
        .expect("issue ticket");
    let deadline = Instant::now() + Duration::from_secs(2);
    let ticket = loop {
        assert!(Instant::now() < deadline, "resume ticket did not arrive");
        if let Some(ticket) = client
            .poll(32, Duration::from_millis(5))
            .into_iter()
            .find_map(|event| match event {
                GameEvent::ResumeTicket { ticket, .. } => Some(ticket),
                _ => None,
            })
        {
            break ticket;
        }
        let _ = server.poll(32, Duration::ZERO);
    };

    let proxy = LossyUdpProxy::start(listener_address);
    let resumed_endpoint = client
        .connect_range_resume(
            GameRangeClientConfig {
                transport: Transport::Udp,
                bind_addr: None,
                remote_addr: proxy.address,
                join_ticket: b"resume-login".to_vec(),
                protocol: GameProtocolRange::new(0x5253_554d, 1, 3),
            },
            old_client,
            ticket.as_bytes(),
        )
        .expect("resume connection");
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut authorized_resume = false;
    while !authorized_resume && Instant::now() < deadline {
        for event in server.poll(32, Duration::from_millis(5)) {
            if let GameEvent::ResumeRequest { session, .. } = event {
                proxy.forward_one_server_packet_then_block();
                server.auth_decide(session, true).expect("authorize resume");
                authorized_resume = true;
            }
        }
        let _ = client.poll(32, Duration::from_millis(5));
    }
    assert!(
        authorized_resume,
        "resume did not reach business authorization"
    );

    // Keep both runtimes driving beyond the v4 negotiation deadline. The resumed route can fail,
    // but the original direct route must not be invalidated before a mapping event is published.
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        assert!(
            !server
                .poll(32, Duration::from_millis(5))
                .iter()
                .any(|event| matches!(event, GameEvent::SessionResumed { .. })),
            "blocked negotiation cannot publish a server mapping"
        );
        assert!(
            !client
                .poll(32, Duration::from_millis(5))
                .iter()
                .any(|event| matches!(event, GameEvent::SessionResumed { endpoint, .. } if *endpoint == resumed_endpoint)),
            "blocked negotiation cannot publish a client mapping"
        );
    }

    client
        .send(old_client, b"client-old-route")
        .expect("old client remains ready");
    await_message(&client, &server, old_server, b"client-old-route");
    server
        .send(old_server, b"server-old-route")
        .expect("old server remains ready");
    await_message(&server, &client, old_client, b"server-old-route");

    client.stop(Duration::ZERO).expect("stop client");
    server.stop(Duration::ZERO).expect("stop server");
}

fn await_message(sender: &GameRuntime, receiver: &GameRuntime, session: u64, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        sender.poll(32, Duration::ZERO);
        if receiver
            .poll(32, Duration::from_millis(5))
            .into_iter()
            .any(|event| {
                matches!(event, GameEvent::Message(message) if message.session == session && message.payload.as_ref() == expected)
            })
        {
            return;
        }
    }
    panic!("message did not arrive on preserved old session");
}

fn run_loss_case(transport: Transport) {
    let server_key = Keypair::generate().expect("server key");
    let client_key = Keypair::generate().expect("client key");
    let server_public = server_key.public.clone();
    let server = GameRuntime::new(GameRuntimeConfig::production()).expect("server runtime");
    let client = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production(),
        ClientSecurity::pinned(client_key, server_public),
    )
    .expect("client runtime");
    let listener = server
        .listen_range(GameRangeServerConfig {
            transport,
            bind_addr: "127.0.0.1:0".parse().expect("bind address"),
            local_key: server_key,
            initial_encryption: true,
            protocol: GameProtocolRange::new(0x4c4f_5353, 2, 8),
        })
        .expect("range listener");
    let proxy = LossyUdpProxy::start(
        server
            .endpoint_local_addr(listener)
            .expect("listener address"),
    );
    let client_endpoint = client
        .connect_range(GameRangeClientConfig {
            transport,
            bind_addr: None,
            remote_addr: proxy.address,
            join_ticket: b"loss-test".to_vec(),
            protocol: GameProtocolRange::new(0x4c4f_5353, 4, 6),
        })
        .expect("range connect");

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut server_session = None;
    let mut client_session = None;
    let mut server_ready_count = 0;
    let mut client_ready_count = 0;
    while (server_session.is_none() || client_session.is_none()) && Instant::now() < deadline {
        for event in server.poll(32, Duration::from_millis(5)) {
            match event {
                GameEvent::AuthRequest { session, .. } => {
                    // Discard one server-side negotiation control after Noise authentication.
                    // ACK-loss behavior is covered deterministically by the range-state tests;
                    // its datagram position depends on transport authentication timing.
                    proxy.drop_second_server_packet();
                    server.auth_decide(session, true).expect("authorize");
                }
                GameEvent::SessionReady { session, .. } => {
                    server_ready_count += 1;
                    server_session = Some(session);
                }
                GameEvent::JoinFailed { .. }
                | GameEvent::ProtocolRejected { .. }
                | GameEvent::ProtocolViolation { .. }
                | GameEvent::SessionClosed { .. } => panic!("server negotiation failed: {event:?}"),
                _ => {}
            }
        }
        for event in client.poll(32, Duration::from_millis(5)) {
            match event {
                GameEvent::SessionReady { endpoint, session } if endpoint == client_endpoint => {
                    client_ready_count += 1;
                    client_session = Some(session)
                }
                GameEvent::JoinFailed { .. }
                | GameEvent::ProtocolRejected { .. }
                | GameEvent::ProtocolViolation { .. }
                | GameEvent::SessionClosed { .. } => panic!("client negotiation failed: {event:?}"),
                _ => {}
            }
        }
    }
    let server_session = server_session.expect("server recovered after packet loss");
    let client_session = client_session.expect("client recovered after packet loss");
    proxy.assert_armed_drops_were_observed();

    // Keep driving both sides beyond multiple retry intervals. A delayed SELECT/ACK/READY must
    // not publish a second readiness event after the state machine has converged.
    let duplicate_deadline = Instant::now() + Duration::from_millis(350);
    while Instant::now() < duplicate_deadline {
        server_ready_count += server
            .poll(32, Duration::from_millis(5))
            .into_iter()
            .filter(|event| matches!(event, GameEvent::SessionReady { .. }))
            .count();
        client_ready_count += client
            .poll(32, Duration::from_millis(5))
            .into_iter()
            .filter(|event| matches!(event, GameEvent::SessionReady { .. }))
            .count();
    }
    assert_eq!(server_ready_count, 1, "server readiness must be idempotent");
    assert_eq!(client_ready_count, 1, "client readiness must be idempotent");
    assert_eq!(
        server
            .selected_protocol_version(server_session)
            .expect("server selected version"),
        6
    );
    assert_eq!(
        client
            .selected_protocol_version(client_session)
            .expect("client selected version"),
        6
    );

    if transport == Transport::Kcp {
        proxy
            .client_to_server_drop_countdown
            .store(0, Ordering::Release);
    }
    client
        .send(client_session, b"survived-loss")
        .expect("send after lossy join");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = false;
    while !received && Instant::now() < deadline {
        received = server
            .poll(32, Duration::from_millis(10))
            .into_iter()
            .any(|event| {
                matches!(event, GameEvent::Message(message) if message.session == server_session && message.payload.as_ref() == b"survived-loss")
            });
        let _ = client.poll(32, Duration::ZERO);
    }
    assert!(
        received,
        "{transport:?} message did not survive configured loss"
    );
    if transport == Transport::Kcp {
        assert_eq!(
            proxy
                .client_to_server_drop_countdown
                .load(Ordering::Acquire),
            usize::MAX
        );
    }

    client.stop(Duration::ZERO).expect("stop client");
    server.stop(Duration::ZERO).expect("stop server");
}
