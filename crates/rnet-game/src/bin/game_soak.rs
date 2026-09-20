//! Local game-facade soak probe; completion alone does not certify deployment SLOs.

use rnet_core::{ErrorCode, Transport};
use rnet_game::{
    GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::io::Write;
use std::time::{Duration, Instant};

struct Config {
    transport: Transport,
    duration: Duration,
    clients: usize,
    payload_bytes: usize,
    rate: u64,
}

struct SoakCounters {
    sent: u64,
    received: u64,
    would_block: u64,
    started: Instant,
    cpu_start: Option<(u64, u64)>,
}

impl Config {
    fn parse() -> Result<Self, Box<dyn std::error::Error>> {
        let args: Vec<String> = std::env::args().collect();
        if args.len() != 6 {
            return Err("usage: game_soak tcp|udp|kcp DURATION_SECONDS CLIENTS PAYLOAD_BYTES RATE_PER_CLIENT".into());
        }
        let transport = match args[1].as_str() {
            "tcp" => Transport::Tcp,
            "udp" => Transport::Udp,
            "kcp" => Transport::Kcp,
            _ => return Err("transport must be tcp, udp, or kcp".into()),
        };
        let duration_seconds: u64 = args[2].parse()?;
        let clients: usize = args[3].parse()?;
        let payload_bytes: usize = args[4].parse()?;
        let rate: u64 = args[5].parse()?;
        if duration_seconds == 0
            || !(1..=4096).contains(&clients)
            || !(12..=65_536).contains(&payload_bytes)
            || (transport == Transport::Udp && payload_bytes > 1024)
            || !(1..=10_000).contains(&rate)
        {
            return Err("probe arguments exceed their safe bounds".into());
        }
        Ok(Self {
            transport,
            duration: Duration::from_secs(duration_seconds),
            clients,
            payload_bytes,
            rate,
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse()?;
    guard_available_memory()?;
    let server_key = Keypair::generate()?;
    let security = ClientSecurity::pinned(Keypair::generate()?, server_key.public.clone());
    let mut settings = GameRuntimeConfig::production();
    settings.network.worker_threads = 4;
    settings.network.max_endpoints = settings.network.max_endpoints.max(config.clients + 8);
    settings.network.max_sessions_per_endpoint = settings
        .network
        .max_sessions_per_endpoint
        .max(config.clients + 8);
    settings.network.max_sessions_per_ip =
        settings.network.max_sessions_per_ip.max(config.clients + 8);
    settings.network.max_pending_handshakes = settings
        .network
        .max_pending_handshakes
        .max(config.clients + 8);
    settings.network.event_queue_capacity = settings
        .network
        .event_queue_capacity
        .max(config.clients.saturating_mul(8));
    settings.network.handshake_burst_per_ip = settings
        .network
        .handshake_burst_per_ip
        .max((config.clients + 8) as u32);
    settings.network.handshake_rate_per_ip = settings
        .network
        .handshake_rate_per_ip
        .max((config.clients + 8) as u32);
    let runtime = GameRuntime::new_with_client_security(settings, security)?;
    let listener = runtime.listen(GameServerConfig {
        transport: config.transport,
        bind_addr: "127.0.0.1:0".parse()?,
        local_key: server_key,
        initial_encryption: true,
        protocol: GameProtocol::new(0x4741_4d45, 1),
    })?;
    let address = runtime.endpoint_local_addr(listener)?;
    let mut client_sessions = Vec::with_capacity(config.clients);
    for index in 0..config.clients {
        let endpoint = runtime.connect(GameClientConfig {
            transport: config.transport,
            bind_addr: None,
            remote_addr: address,
            join_ticket: (index as u32).to_be_bytes().to_vec(),
            protocol: GameProtocol::new(0x4741_4d45, 1),
        })?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ready = None;
        while ready.is_none() {
            if Instant::now() >= deadline {
                return Err(format!("join timed out for client {index}").into());
            }
            for event in runtime.poll(256, Duration::from_millis(5)) {
                match event {
                    GameEvent::AuthRequest { session, .. } => runtime.auth_decide(session, true)?,
                    GameEvent::SessionReady {
                        endpoint: opened,
                        session,
                    } if opened == endpoint => ready = Some(session),
                    GameEvent::ProtocolViolation { .. }
                    | GameEvent::ProtocolRejected { .. }
                    | GameEvent::JoinFailed { .. }
                    | GameEvent::SessionClosed { .. } => {
                        return Err(format!("join failed: {event:?}").into())
                    }
                    _ => {}
                }
            }
        }
        client_sessions.push(ready.expect("join loop completed"));
    }

    let tick = Duration::from_nanos(1_000_000_000 / config.rate);
    let mut next_send = vec![Instant::now(); config.clients];
    let mut sequences = vec![0_u64; config.clients];
    let mut payload = vec![0x5a_u8; config.payload_bytes];
    let mut counters = SoakCounters {
        sent: 0,
        received: 0,
        would_block: 0,
        started: Instant::now(),
        cpu_start: cpu_ticks(),
    };
    let deadline = counters.started + config.duration;
    let mut next_report = counters.started + Duration::from_secs(60);
    let mut next_memory_check = counters.started;
    while Instant::now() < deadline {
        let now = Instant::now();
        if now >= next_memory_check {
            guard_available_memory()?;
            next_memory_check = now + Duration::from_secs(1);
        }
        for (index, session) in client_sessions.iter().copied().enumerate() {
            if now < next_send[index] {
                continue;
            }
            payload[..4].copy_from_slice(&(index as u32).to_be_bytes());
            payload[4..12].copy_from_slice(&sequences[index].to_be_bytes());
            match runtime.send(session, &payload) {
                Ok(()) => {
                    counters.sent = counters.sent.saturating_add(1);
                    sequences[index] = sequences[index].wrapping_add(1);
                }
                Err(error) if error.code() == ErrorCode::WouldBlock => {
                    counters.would_block = counters.would_block.saturating_add(1)
                }
                Err(error) => return Err(error.into()),
            }
            next_send[index] = now + tick;
        }
        counters.received = counters
            .received
            .saturating_add(poll_messages(&runtime, listener)?);
        if now >= next_report {
            report(&config, &runtime, &counters, "running")?;
            next_report = now + Duration::from_secs(60);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let drain_deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < drain_deadline {
        counters.received = counters
            .received
            .saturating_add(poll_messages(&runtime, listener)?);
    }
    report(&config, &runtime, &counters, "completed")?;
    runtime.stop(Duration::ZERO)?;
    Ok(())
}

fn poll_messages(runtime: &GameRuntime, listener: u64) -> Result<u64, Box<dyn std::error::Error>> {
    let mut received = 0_u64;
    for event in runtime.poll(4096, Duration::ZERO) {
        match event {
            GameEvent::Message(message) if message.endpoint == listener => {
                if message.payload.len() < 12 {
                    return Err("truncated probe message".into());
                }
                received = received.saturating_add(1);
            }
            GameEvent::SessionClosed { .. }
            | GameEvent::ProtocolViolation { .. }
            | GameEvent::JoinFailed { .. } => {
                return Err(format!("soak session failed: {event:?}").into())
            }
            _ => {}
        }
    }
    Ok(received)
}

fn report(
    config: &Config,
    runtime: &GameRuntime,
    counters: &SoakCounters,
    status: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let metrics = runtime.metrics_snapshot();
    let heartbeat = runtime.heartbeat_metrics_snapshot();
    let transport = match config.transport {
        Transport::Tcp => "tcp",
        Transport::Udp => "udp",
        Transport::Kcp => "kcp",
    };
    let cpu_percent = counters
        .cpu_start
        .zip(cpu_ticks())
        .map(|(start, end)| {
            let process = end.0.saturating_sub(start.0) as f64;
            let total = end.1.saturating_sub(start.1) as f64;
            let cpus = std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1) as f64;
            if total == 0.0 {
                0.0
            } else {
                process / total * cpus * 100.0
            }
        })
        .unwrap_or(0.0);
    println!(
        "status={status} transport={transport} elapsed_seconds={:.3} clients={} payload_bytes={} rate_per_client={} sent={} received={} would_block={} rtt_samples={} rtt_p95_us={} rtt_p99_us={} heartbeat_timeouts={} event_drops={} protocol_errors={} cpu_percent={cpu_percent:.1} rss_kib={}",
        counters.started.elapsed().as_secs_f64(), config.clients, config.payload_bytes, config.rate, counters.sent, counters.received,
        counters.would_block, heartbeat.rtt.sample_count, heartbeat.rtt.p95_us, heartbeat.rtt.p99_us,
        heartbeat.timeouts, metrics.events_dropped, metrics.protocol_errors, rss_kib().unwrap_or(0),
    );
    std::io::stdout().flush()?;
    Ok(())
}

fn cpu_ticks() -> Option<(u64, u64)> {
    let process = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<_> = process.rsplit_once(") ")?.1.split_whitespace().collect();
    let process_ticks = fields
        .get(11)?
        .parse::<u64>()
        .ok()?
        .saturating_add(fields.get(12)?.parse::<u64>().ok()?);
    let system = std::fs::read_to_string("/proc/stat").ok()?;
    let total_ticks = system
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse::<u64>().ok())
        .sum();
    Some((process_ticks, total_ticks))
}

fn rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn guard_available_memory() -> Result<(), Box<dyn std::error::Error>> {
    const MINIMUM_KIB: u64 = 15 * 1024 * 1024;
    let available = std::fs::read_to_string("/proc/meminfo")?
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|value| value.split_whitespace().next())
        .ok_or("MemAvailable is unavailable")?
        .parse::<u64>()?;
    if available < MINIMUM_KIB {
        return Err(format!("soak stopped: MemAvailable {available} KiB is below 15 GiB").into());
    }
    Ok(())
}
