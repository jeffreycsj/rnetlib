use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    BoundedLogger, GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig,
    GameServerConfig, LogRecord, LoggerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// Both directions use the same source address, including injected invalid datagrams.
// Dropping packets leaves the real endpoint sockets open and their timers running.
struct Proxy {
    socket: Arc<UdpSocket>,
    blocked: Arc<AtomicU8>,
    client_packets_dropped: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Proxy {
    fn new(server: SocketAddr) -> Self {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap());
        socket
            .set_read_timeout(Some(Duration::from_millis(10)))
            .unwrap();
        let blocked = Arc::new(AtomicU8::new(0));
        let client_packets_dropped = Arc::new(AtomicUsize::new(0));
        let dropped = client_packets_dropped.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let (reader, drop_packets, stop) = (socket.clone(), blocked.clone(), stopped.clone());
        let worker = std::thread::spawn(move || {
            let mut client = None;
            let mut buffer = [0; 65_536];
            while !stop.load(Ordering::Acquire) {
                let (len, source) = match reader.recv_from(&mut buffer) {
                    Ok(packet) => packet,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => panic!("proxy receive failed: {error}"),
                };
                let mode = drop_packets.load(Ordering::Acquire);
                if mode == 1 || (mode == 2 && source != server) {
                    if source != server {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    continue;
                }
                let destination = if source == server {
                    client.expect("server cannot reply before the client")
                } else {
                    client = Some(source);
                    server
                };
                reader.send_to(&buffer[..len], destination).unwrap();
            }
        });
        Self {
            socket,
            blocked,
            client_packets_dropped,
            stopped,
            worker: Some(worker),
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let result = self.worker.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

struct Connection {
    runtime: GameRuntime,
    proxy: Proxy,
    logs: mpsc::Receiver<LogRecord>,
    server_addr: SocketAddr,
    listener: u64,
    server: u64,
    client: u64,
}

impl Connection {
    fn new(transport: Transport, idle_timeout: Duration) -> Self {
        let (tx, logs) = mpsc::channel();
        let logger = BoundedLogger::new(LoggerConfig::default(), move |record| {
            let _ = tx.send(record);
        })
        .unwrap();
        let key = Keypair::generate().unwrap();
        let mut config = GameRuntimeConfig::production();
        config.network.handshake_timeout = Duration::from_secs(1);
        config.network.datagram_idle_timeout = idle_timeout;
        let runtime = GameRuntime::new_with_client_security(
            config,
            ClientSecurity::pinned(Keypair::generate().unwrap(), key.public.clone()),
        )
        .unwrap()
        .with_logger(logger);
        let listener = runtime
            .listen(GameServerConfig {
                transport,
                bind_addr: "127.0.0.1:0".parse().unwrap(),
                local_key: key,
                initial_encryption: true,
                protocol: GameProtocol::new(99, 1),
            })
            .unwrap();
        let server_addr = runtime.endpoint_local_addr(listener).unwrap();
        let proxy = Proxy::new(server_addr);
        runtime
            .connect(GameClientConfig {
                transport,
                bind_addr: None,
                remote_addr: proxy.socket.local_addr().unwrap(),
                join_ticket: b"secret-datagram-ticket".to_vec(),
                protocol: GameProtocol::new(99, 1),
            })
            .unwrap();
        let mut connection = Self {
            runtime,
            proxy,
            logs,
            server_addr,
            listener,
            server: 0,
            client: 0,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while (connection.server == 0 || connection.client == 0) && Instant::now() < deadline {
            for event in connection.runtime.poll(32, Duration::from_millis(10)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => {
                        connection.runtime.auth_decide(session, true).unwrap();
                    }
                    GameEvent::SessionReady { endpoint, session } if endpoint == listener => {
                        connection.server = session;
                    }
                    GameEvent::SessionReady { session, .. } => connection.client = session,
                    _ => {}
                }
            }
        }
        assert_ne!(connection.server, 0, "{transport:?} server not ready");
        assert_ne!(connection.client, 0, "{transport:?} client not ready");
        connection
    }

    fn expect_timeout(&self, phase: &str, cause: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut closed = false;
        while !closed && Instant::now() < deadline {
            for event in self.runtime.poll(32, Duration::from_millis(10)) {
                if let GameEvent::SessionClosed {
                    session, reason, ..
                } = event
                {
                    if session == self.server {
                        assert_eq!(reason, ErrorCode::Timeout);
                        closed = true;
                    }
                }
            }
        }
        assert!(closed, "server timeout not reported");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let record = self
                .logs
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            assert!(!record.message.contains("secret-datagram-ticket"));
            if record.event_name == "game_session_closed" && record.session == self.server {
                assert_eq!(record.endpoint, self.listener);
                assert_eq!(record.error_code, ErrorCode::Timeout);
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

#[test]
fn udp_and_kcp_idle_timeout_logs_local_cause() {
    for transport in [Transport::Udp, Transport::Kcp] {
        let connection = Connection::new(transport, Duration::from_secs(1));
        connection.proxy.blocked.store(1, Ordering::Release);
        connection.expect_timeout("phase=datagram_idle", "idle timeout");
    }
}

#[test]
fn udp_and_kcp_security_timeout_logs_missing_ack() {
    for transport in [Transport::Udp, Transport::Kcp] {
        let connection = Connection::new(transport, Duration::from_secs(30));
        // Deliver the server's proposal but suppress client replies, not both directions.
        connection.proxy.blocked.store(2, Ordering::Release);
        connection.runtime.rekey(connection.server).unwrap();
        connection.expect_timeout(
            "phase=datagram_security_transition",
            "acknowledgement timed out",
        );
        assert!(
            connection
                .proxy
                .client_packets_dropped
                .load(Ordering::Relaxed)
                > 0
        );
    }
}

#[test]
fn malformed_datagrams_do_not_close_ready_sessions_or_flood_game_logs() {
    for transport in [Transport::Udp, Transport::Kcp] {
        let connection = Connection::new(transport, Duration::from_secs(30));
        for _ in 0..64 {
            connection
                .proxy
                .socket
                .send_to(b"bad", connection.server_addr)
                .unwrap();
        }
        connection
            .runtime
            .send(connection.client, b"private-game-payload")
            .unwrap();
        let mut received = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !received && Instant::now() < deadline {
            for event in connection.runtime.poll(32, Duration::from_millis(10)) {
                match event {
                    GameEvent::Message(message) if message.session == connection.server => {
                        assert_eq!(message.payload.as_ref(), b"private-game-payload");
                        received = true;
                    }
                    GameEvent::SessionClosed { .. } | GameEvent::ProtocolViolation { .. } => {
                        panic!("malformed datagrams must not become game events: {event:?}");
                    }
                    _ => {}
                }
            }
        }
        assert!(
            received,
            "{transport:?} valid data not received after bad datagrams"
        );
        assert!(connection.runtime.metrics_snapshot().protocol_errors > 0);
        connection.runtime.stop(Duration::ZERO).unwrap();
        drop(connection.runtime); // Join the logger before inspecting all queued records.
        let records: Vec<_> = connection.logs.try_iter().collect();
        assert!(records.len() < 32, "malformed datagrams flooded game logs");
        assert!(records
            .iter()
            .all(|record| !record.message.contains("private-game-payload")));
    }
}
