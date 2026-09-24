use super::run_adaptive_tcp_listener;
use crate::{
    ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig, SecurityMode, ServerConfig,
};
use rnet_core::{ErrorCode, Event, EventType, Handle, Transport};
use rnet_security::Keypair;
use std::io::Read;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::{Duration, Instant};

fn start_listener(
    runtime: &NetworkRuntime,
    key: Keypair,
) -> (Handle, TcpListener, tokio::task::JoinHandle<()>) {
    let fault_socket = TcpListener::bind("127.0.0.1:0").unwrap();
    fault_socket.set_nonblocking(true).unwrap();
    let listener = {
        let _entered = runtime.runtime.enter();
        tokio::net::TcpListener::from_std(fault_socket.try_clone().unwrap()).unwrap()
    };
    let endpoint = runtime
        .insert_endpoint(fault_socket.local_addr().unwrap(), Transport::Tcp)
        .unwrap();
    let task = runtime.runtime.spawn(run_adaptive_tcp_listener(
        runtime.shared.clone(),
        endpoint,
        listener,
        key,
        SecurityMode::Encrypted,
    ));
    runtime
        .set_endpoint_abort(endpoint, task.abort_handle())
        .unwrap();
    (endpoint, fault_socket, task)
}

fn connect_ready(runtime: &NetworkRuntime, listener: Handle) -> (Handle, Handle) {
    let client = runtime
        .connect(ClientConfig {
            transport: Transport::Tcp,
            bind_addr: None,
            remote_addr: runtime.endpoint_local_addr(listener).unwrap(),
            join_payload: Vec::new(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut server_session, mut client_session) = (0, 0);
    while Instant::now() < deadline && (server_session == 0 || client_session == 0) {
        for event in runtime.poll_events(64, Duration::from_millis(10)) {
            match event.event_type {
                EventType::AuthRequest => runtime.auth_decide(event.session, true).unwrap(),
                EventType::SessionOpened if event.endpoint == listener => {
                    server_session = event.session
                }
                EventType::SessionOpened if event.endpoint == client => {
                    client_session = event.session
                }
                _ => {}
            }
        }
    }
    assert_ne!(server_session, 0);
    assert_ne!(client_session, 0);
    (server_session, client_session)
}

// shutdown on a duplicate Linux listening socket wakes the real accept with a local error.
// No unsafe descriptor manipulation or production-only fault injection is needed.
#[test]
fn fatal_accept_reclaims_routes_even_when_events_are_full_and_other_listener_survives() {
    for full in [false, true] {
        let key = Keypair::generate().unwrap();
        let runtime = NetworkRuntime::new_with_client_security(
            RuntimeConfig {
                event_queue_capacity: 32,
                ..RuntimeConfig::production()
            },
            Some(ClientSecurity::pinned(
                Keypair::generate().unwrap(),
                key.public.clone(),
            )),
        )
        .unwrap();
        let (endpoint, fault_socket, task) = start_listener(&runtime, key.clone());
        let (session, _) = connect_ready(&runtime, endpoint);
        let other = runtime
            .listen(ServerConfig {
                transport: Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: key,
                initial_security: SecurityMode::Encrypted,
            })
            .unwrap();
        let (_, other_client) = connect_ready(&runtime, other);
        let mut raw = TcpStream::connect(fault_socket.local_addr().unwrap()).unwrap();
        raw.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while runtime.metrics_snapshot().pending_handshakes == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(runtime.metrics_snapshot().pending_handshakes, 1);
        runtime.poll_events(128, Duration::ZERO);
        if full {
            while runtime
                .shared
                .events
                .try_push(Event::simple(EventType::RuntimeStarted))
                .is_ok()
            {}
        }
        socket2::SockRef::from(&fault_socket)
            .shutdown(Shutdown::Both)
            .unwrap();
        runtime.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap();
        });
        assert_eq!(
            runtime
                .endpoint_local_addr(endpoint)
                .err()
                .map(|e| e.code()),
            Some(ErrorCode::InvalidHandle)
        );
        assert_eq!(
            runtime.send_payload(session, b"closed").unwrap_err().code(),
            ErrorCode::InvalidHandle
        );
        assert_eq!(runtime.metrics_snapshot().pending_handshakes, 0);
        let events = runtime.poll_events(128, Duration::ZERO);
        if full {
            assert!(runtime.metrics_snapshot().lifecycle_events_rejected >= 3);
        } else {
            let affected: Vec<_> = events.iter().filter(|e| e.endpoint == endpoint).collect();
            assert_eq!(
                affected
                    .iter()
                    .filter(|e| e.event_type == EventType::EndpointError)
                    .count(),
                1
            );
            assert_eq!(
                affected
                    .iter()
                    .filter(|e| e.event_type == EventType::SessionClosed)
                    .count(),
                2
            );
            assert!(affected.iter().all(|e| e.status == ErrorCode::IoError));
            assert!(affected
                .iter()
                .all(|e| String::from_utf8_lossy(&e.data).contains("phase=tcp_accept")));
        }
        assert_eq!(
            raw.read(&mut [0]).unwrap(),
            0,
            "pending handshake socket was not dropped"
        );
        runtime
            .send_payload(other_client, b"other listener alive")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut received = false;
        while !received && Instant::now() < deadline {
            received = runtime
                .poll_events(64, Duration::from_millis(10))
                .iter()
                .any(|e| {
                    e.event_type == EventType::Message
                        && e.endpoint == other
                        && e.data == b"other listener alive"
                });
        }
        assert!(received);
        connect_ready(&runtime, other);
        runtime.stop(Duration::ZERO).unwrap();
    }
}

#[test]
fn fatal_accept_cancels_silent_handshake_without_waiting_for_its_timeout() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        handshake_timeout: Duration::from_secs(30),
        ..RuntimeConfig::production()
    })
    .unwrap();
    let (_, fault_socket, task) = start_listener(&runtime, Keypair::generate().unwrap());
    let mut raw = TcpStream::connect(fault_socket.local_addr().unwrap()).unwrap();
    raw.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while runtime.metrics_snapshot().pending_handshakes == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(runtime.metrics_snapshot().pending_handshakes, 1);
    socket2::SockRef::from(&fault_socket)
        .shutdown(Shutdown::Both)
        .unwrap();
    runtime.runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
    });
    assert_eq!(
        raw.read(&mut [0]).unwrap(),
        0,
        "silent handshake task outlived its listener"
    );
}

#[test]
fn entering_draining_does_not_cancel_existing_handshake_tasks() {
    let runtime = NetworkRuntime::new(RuntimeConfig {
        handshake_timeout: Duration::from_secs(30),
        ..RuntimeConfig::production()
    })
    .unwrap();
    let (endpoint, listener, _task) = start_listener(&runtime, Keypair::generate().unwrap());
    let mut pending = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    pending
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while runtime.metrics_snapshot().pending_handshakes == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(runtime.metrics_snapshot().pending_handshakes, 1);
    // This is the first transition performed by stop(drain_timeout). Wake an outstanding
    // accept so the listener observes it while the existing connection remains pending.
    runtime.shared.state.begin_draining().unwrap();
    let _wake = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let error = pending
        .read(&mut [0])
        .expect_err("draining cancelled a live handshake before its deadline");
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    assert!(runtime.endpoint_local_addr(endpoint).is_ok());
    runtime.stop(Duration::ZERO).unwrap();
    pending
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(pending.read(&mut [0]).unwrap(), 0);
}
