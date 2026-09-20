//! Local encrypted throughput probe for capacity planning, not a substitute for production soak.

use rnet_core::{ErrorCode, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientSecurity, HostClientConfig, NetworkRuntime, RuntimeConfig, SecurityMode, ServerConfig,
};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let transport = match std::env::args().nth(1).as_deref() {
        Some("udp") => Transport::Udp,
        Some("kcp") => Transport::Kcp,
        Some("tcp") | None => Transport::Tcp,
        Some(other) => return Err(format!("unknown transport: {other}").into()),
    };
    let messages = std::env::args()
        .nth(2)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(100_000_u64);
    let payload_len = std::env::args()
        .nth(3)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(if transport == Transport::Udp {
            1024
        } else {
            4096
        });
    let max_duration = Duration::from_secs(
        std::env::args()
            .nth(4)
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(300_u64),
    );

    let server_key = Keypair::generate()?;
    let client_security = ClientSecurity::pinned(Keypair::generate()?, server_key.public.clone());
    let runtime = NetworkRuntime::new_with_client_security(
        RuntimeConfig {
            event_queue_capacity: 65_536,
            write_queue_capacity: 4_096,
            max_sessions_per_endpoint: 65_536,
            ..RuntimeConfig::default()
        },
        Some(client_security),
    )?;
    let listener = runtime.listen(ServerConfig {
        transport,
        bind_addr: "127.0.0.1:0".parse()?,
        local_key: server_key,
        initial_security: SecurityMode::Encrypted,
    })?;
    let client = runtime.connect_host(HostClientConfig {
        transport,
        host: "localhost".to_owned(),
        port: runtime.endpoint_local_addr(listener)?.port(),
        join_payload: b"load-probe".to_vec(),
    })?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut client_session = 0;
    while Instant::now() < deadline && client_session == 0 {
        for event in runtime.poll_events(256, Duration::from_millis(10)) {
            match event.event_type {
                EventType::AuthRequest => runtime.auth_decide(event.session, true)?,
                EventType::SessionOpened if event.endpoint == client => {
                    client_session = event.session;
                }
                _ => {}
            }
        }
    }
    if client_session == 0 {
        return Err("session establishment timed out".into());
    }

    let payload = vec![0x5a; payload_len];
    let cpu_start = cpu_ticks();
    let started = Instant::now();
    let mut sent = 0_u64;
    let mut received = 0_u64;
    while received < messages && started.elapsed() < max_duration {
        while sent < messages {
            match runtime.send(client_session, 1, &payload) {
                Ok(()) => sent += 1,
                Err(error) if error.code() == ErrorCode::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        for event in runtime.poll_events(4096, Duration::from_millis(1)) {
            if event.event_type == EventType::Message && event.endpoint == listener {
                received += 1;
            }
        }
    }
    let elapsed = started.elapsed();
    let cpu_percent = cpu_start
        .zip(cpu_ticks())
        .map(|(start, end)| cpu_percent(start, end))
        .unwrap_or(0.0);
    println!(
        "transport={transport:?} target_messages={messages} sent={sent} received={received} payload={payload_len} elapsed_seconds={:.3} target_reached={} rate={:.0}msg/s throughput={:.1}MiB/s cpu_percent={cpu_percent:.1} rss_kib={}",
        elapsed.as_secs_f64(),
        received == messages,
        received as f64 / elapsed.as_secs_f64(),
        (received as f64 * payload_len as f64 / 1_048_576.0) / elapsed.as_secs_f64(),
        rss_kib().unwrap_or(0),
    );
    for metric in runtime.latency_snapshot() {
        if metric.latency.sample_count != 0 {
            println!(
                "latency={:?} samples={} p50={}us p90={}us p95={}us p99={}us p999={}us max={}us",
                metric.kind,
                metric.latency.sample_count,
                metric.latency.p50_us,
                metric.latency.p90_us,
                metric.latency.p95_us,
                metric.latency.p99_us,
                metric.latency.p999_us,
                metric.latency.max_us,
            );
        }
    }
    let metrics = runtime.metrics_snapshot();
    println!(
        "metrics frames_sent={} frames_received={} bytes_sent={} bytes_received={} events_dropped={} send_would_block={} protocol_errors={} admission_rejected={} queued_send_bytes={} peak_queued_send_bytes={} queued_event_bytes={}",
        metrics.frames_sent,
        metrics.frames_received,
        metrics.bytes_sent,
        metrics.bytes_received,
        metrics.events_dropped,
        metrics.send_would_block,
        metrics.protocol_errors,
        metrics.admission_rejected,
        metrics.queued_send_bytes,
        metrics.peak_queued_send_bytes,
        metrics.queued_event_bytes,
    );
    runtime.stop(Duration::ZERO)?;
    Ok(())
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

fn cpu_percent(start: (u64, u64), end: (u64, u64)) -> f64 {
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
}
