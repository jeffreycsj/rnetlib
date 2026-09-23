use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    BoundedLogger, GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig,
    GameServerConfig, LoggerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

// Forward complete wire records so corruption is injected between records, never in a
// timing-dependent partial write. The proxy never decodes credentials or business bytes.
fn forward(mut input: TcpStream, mut output: TcpStream, fault: Option<Arc<AtomicU8>>) {
    loop {
        let mut prefix = [0; 4];
        if input.read_exact(&mut prefix).is_err() {
            break;
        }
        let len = u32::from_be_bytes(prefix) as usize;
        assert!(len < 1_048_576, "bounded test proxy");
        let mut body = vec![0; len];
        if input.read_exact(&mut body).is_err() {
            break;
        }
        let fault = fault
            .as_ref()
            .map_or(0, |flag| flag.load(Ordering::Acquire));
        if fault == 2 {
            continue; // Suppress acknowledgements while leaving both TCP streams open.
        }
        if fault == 1 {
            // Zero-length transport records are invalid, independent of encryption mode.
            let _ = output.write_all(&[0; 4]);
            break;
        }
        if output
            .write_all(&prefix)
            .and_then(|_| output.write_all(&body))
            .is_err()
        {
            break;
        }
    }
    let _ = output.shutdown(Shutdown::Write);
}

#[test]
fn established_tcp_logs_eof_invalid_framing_and_security_timeout_without_payloads() {
    for fault_mode in [0, 1, 2] {
        let (tx, rx) = mpsc::channel();
        let logger = BoundedLogger::new(LoggerConfig::default(), move |record| {
            let _ = tx.send(record);
        })
        .unwrap();
        let key = Keypair::generate().unwrap();
        let mut config = GameRuntimeConfig::production();
        config.network.handshake_timeout = Duration::from_secs(1);
        let runtime = GameRuntime::new_with_client_security(
            config,
            ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
        )
        .unwrap()
        .with_logger(logger);
        let listener = runtime
            .listen(GameServerConfig {
                transport: Transport::Tcp,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: key,
                initial_encryption: true,
                protocol: GameProtocol::new(99, 1),
            })
            .unwrap();
        let server_addr = runtime.endpoint_local_addr(listener).unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let remote_addr = proxy.local_addr().unwrap();
        let corrupt = Arc::new(AtomicU8::new(0));
        let flag = corrupt.clone();
        let worker = std::thread::spawn(move || {
            let (client, _) = proxy.accept().unwrap();
            let server = TcpStream::connect(server_addr).unwrap();
            for socket in [&client, &server] {
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
            }
            let client_read = client.try_clone().unwrap();
            let server_write = server.try_clone().unwrap();
            let incoming =
                std::thread::spawn(move || forward(client_read, server_write, Some(flag)));
            forward(server, client, None);
            incoming.join().unwrap();
        });
        let connector = runtime
            .connect(GameClientConfig {
                transport: Transport::Tcp,
                bind_addr: None,
                remote_addr,
                join_ticket: b"secret-close-ticket".to_vec(),
                protocol: GameProtocol::new(99, 1),
            })
            .unwrap();
        let (mut server_session, mut client_session) = (0, 0);
        let deadline = Instant::now() + Duration::from_secs(5);
        while (server_session == 0 || client_session == 0) && Instant::now() < deadline {
            for event in runtime.poll(32, Duration::from_millis(10)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        runtime.auth_decide(session, true).unwrap()
                    }
                    GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                        server_session = session
                    }
                    GameEvent::SessionReady { session, .. } => client_session = session,
                    _ => {}
                }
            }
        }
        assert_ne!(server_session, 0);
        assert_ne!(client_session, 0);
        let (endpoint, session, code, phase, cause) = if fault_mode == 1 {
            corrupt.store(1, Ordering::Release);
            runtime
                .send(client_session, b"private-game-payload")
                .unwrap();
            (
                listener,
                server_session,
                ErrorCode::ProtocolError,
                "phase=tcp_framing",
                "empty TCP record",
            )
        } else if fault_mode == 2 {
            corrupt.store(2, Ordering::Release);
            runtime.rekey(server_session).unwrap();
            (
                listener,
                server_session,
                ErrorCode::Timeout,
                "phase=tcp_security_transition",
                "acknowledgement timed out",
            )
        } else {
            runtime.close_session(server_session).unwrap();
            (
                connector,
                client_session,
                ErrorCode::IoError,
                "phase=tcp_read",
                "peer closed",
            )
        };
        let mut closed = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !closed && Instant::now() < deadline {
            for event in runtime.poll(32, Duration::from_millis(10)) {
                if let GameEvent::SessionClosed {
                    session: got,
                    reason,
                    ..
                } = event
                {
                    if got == session {
                        assert_eq!(reason, code);
                        closed = true;
                    }
                }
            }
        }
        assert!(closed);
        runtime.stop(Duration::ZERO).unwrap();
        worker.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let record = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            assert!(!record.message.contains("private-game-payload"));
            assert!(!record.message.contains("secret-close-ticket"));
            if record.event_name == "game_session_closed" && record.session == session {
                assert_eq!(record.endpoint, endpoint);
                assert_eq!(record.error_code, code);
                assert!(
                    record.message.contains(phase),
                    "missing phase: {}",
                    record.message
                );
                assert!(
                    record.message.contains(cause),
                    "missing cause: {}",
                    record.message
                );
                break;
            }
        }
    }
}
