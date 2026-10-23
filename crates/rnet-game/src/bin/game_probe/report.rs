//! Cumulative per-process evidence; no cross-process percentile averaging or clock subtraction.

use super::{
    config::{Config, Mode},
    wire::Disposition,
    Result,
};
use ring::digest::{Context, SHA256};
use rnet_game::GameRuntime;
use rnet_observe::LatencyHistogram;
use rnet_transport::LatencyKind;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

#[derive(Default)]
pub struct Stats {
    /// All scheduled client offers, including ticks the generator could not execute.
    pub offered: u64,
    /// Data accepted by the client game API; not proof of transport delivery.
    pub admitted: u64,
    /// Offers not admitted, including scheduling misses and local pressure.
    pub rejected: u64,
    /// Rejected subset caused by generator lateness, rather than network backpressure.
    pub schedule_missed: u64,
    /// WouldBlock attempts, including echo/finish retries; may exceed rejected offers.
    pub would_block: u64,
    /// Rejected offers caused by the client timestamp budget.
    pub inflight_rejected: u64,
    /// Verified unique data visible inside the bounded sequence window.
    pub received: u64,
    /// Server data echoes admitted by the API, including partial batches before failure.
    pub echoed: u64,
    /// UDP repeated sequences still identifiable inside the receive window.
    pub duplicates: u64,
    /// UDP newly observed data behind the highest seen sequence; included in received.
    pub reordered: u64,
    /// UDP packets outside history; uniqueness cannot be proven, so excluded from received.
    pub too_old: u64,
    /// Newly counted echoes without a retained local timestamp; excluded from RTT samples.
    pub late: u64,
    /// Invalid credentials, duplicate identities, or exhausted distinct-flow admission.
    pub auth_rejected: u64,
    /// Server: valid finish requests; client: verified finish ACKs. Not interchangeable.
    pub finished_flows: u64,
    /// Server retry messages or client outstanding timestamps; separate from runtime queues.
    pub pending_messages: usize,
    /// Server retry payload bytes only; clients retain timestamps rather than payload copies.
    pub pending_bytes: usize,
    /// Cumulative local admission-to-echo latency; no remote clock is subtracted.
    pub rtt: LatencyHistogram,
    started: Option<Instant>,
    cpu_start: Option<(u64, u64, u64)>,
    binary_hash: String,
    package_hash: String,
}

impl Stats {
    pub fn new() -> Result<Self> {
        let executable = std::env::current_exe()?;
        let manifest = executable
            .parent()
            .and_then(Path::parent)
            .map(|path| path.join("SHA256SUMS"));
        Ok(Self {
            started: Some(Instant::now()),
            cpu_start: cpu_ticks(),
            binary_hash: hash_file(&executable)?,
            package_hash: manifest
                .filter(|path| path.is_file())
                .map(|path| hash_file(&path))
                .transpose()?
                .unwrap_or_else(|| "unavailable".into()),
            ..Self::default()
        })
    }

    pub fn disposition(&mut self, disposition: Disposition) -> bool {
        match disposition {
            Disposition::New => {
                self.received += 1;
                true
            }
            Disposition::Reordered => {
                self.received += 1;
                self.reordered += 1;
                true
            }
            Disposition::Duplicate => {
                self.duplicates += 1;
                false
            }
            Disposition::TooOld => {
                self.too_old += 1;
                false
            }
        }
    }

    pub fn check_slo(&self, config: &Config) -> Result<()> {
        if let Some(limit) = config.max_p99_us {
            let rtt = self.rtt.snapshot();
            if rtt.sample_count == 0 || rtt.p99_us > limit {
                return Err("rtt_p99_SLO_failed_or_unsampled".into());
            }
        }
        if let Some(limit) = config.max_unobserved_ppm {
            if self.admitted == 0
                || u128::from(self.admitted.saturating_sub(self.received)) * 1_000_000
                    > u128::from(limit) * u128::from(self.admitted)
            {
                return Err("echo_unobserved_SLO_failed_or_unsampled".into());
            }
        }
        Ok(())
    }

    pub fn snapshot(&self, runtime: &GameRuntime) -> String {
        let metrics = runtime.metrics_snapshot();
        let scheduler = runtime.scheduled_queue_snapshot();
        let heartbeat = runtime.heartbeat_metrics_snapshot();
        let latency = runtime.latency_snapshot();
        let send = latency
            .iter()
            .find(|entry| entry.kind == LatencyKind::SendQueue)
            .unwrap()
            .latency;
        let event = latency
            .iter()
            .find(|entry| entry.kind == LatencyKind::EventQueue)
            .unwrap()
            .latency;
        let reasons = metrics
            .session_closed_by_reason
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let cpu = self
            .cpu_start
            .zip(cpu_ticks())
            .map(|(start, end)| {
                let total = end.1.saturating_sub(start.1);
                if total == 0 {
                    0.0
                } else {
                    end.0.saturating_sub(start.0) as f64 / total as f64 * end.2 as f64 * 100.0
                }
            })
            .map(|value| format!("{value:.2}"))
            .unwrap_or_else(|| "unavailable".into());
        format!("scheduled_messages={} scheduled_bytes={} scheduled_rejected={} scheduled_failed={} scheduled_p99_us={} transport_send_bytes={} transport_event_bytes={} send_queue_samples={} send_queue_p99_us={} event_queue_samples={} event_queue_p99_us={} heartbeat_samples={} heartbeat_p99_us={} heartbeat_timeouts={} sessions={} pending_handshakes={} closed_by_reason={} event_drops={} lifecycle_rejected={} protocol_errors={} cpu_percent={} rss_kib={}",
            scheduler.queued_messages, scheduler.queued_bytes, scheduler.admission_rejected, scheduler.send_failed,
            scheduler.queue_delay.p99_us, metrics.queued_send_bytes, metrics.queued_event_bytes,
            send.sample_count, send.p99_us, event.sample_count, event.p99_us, heartbeat.rtt.sample_count,
            heartbeat.rtt.p99_us, heartbeat.timeouts, metrics.current_sessions, metrics.pending_handshakes,
            reasons, metrics.events_dropped, metrics.lifecycle_events_rejected, metrics.protocol_errors,
            cpu, rss_kib().map(|value| value.to_string()).unwrap_or_else(|| "unavailable".into()))
    }

    pub fn emit(&self, config: &Config, snapshot: &str, status: &str, reason: &str) -> Result<()> {
        let rtt = self.rtt.snapshot();
        let generator = match config.mode {
            Mode::Client { generator, .. } => generator,
            _ => 0,
        };
        let slo = if config.max_p99_us.is_none() && config.max_unobserved_ppm.is_none() {
            "not_evaluated"
        } else if status == "completed" {
            "passed"
        } else if status == "running" {
            "pending"
        } else {
            "failed"
        };
        println!("status={status} role={} transport={} run_id={} generator_id={} elapsed_ms={} duration_ms={} drain_ms={} join_timeout_ms={} connections={} payload_bytes={} rate_per_stream={} queue_messages_limit={} queue_bytes_limit={} inflight_limit={} offered={} admitted={} rejected={} schedule_missed={} would_block={} inflight_rejected={} received={} echoed={} echo_unobserved={} duplicates={} reordered={} too_old={} late={} auth_rejected={} finished_flows={} pending_messages={} pending_bytes={} rtt_samples={} rtt_p95_us={} rtt_p99_us={} rtt_p999_us={} rtt_max_us={} slo={} max_rtt_p99_us={} max_unobserved_ppm={} binary_sha256={} package_manifest_sha256={} reason={} {}",
            config.role(), config.transport_name(), config.run, generator, self.started.map(|start| start.elapsed().as_millis()).unwrap_or(0),
            config.duration.as_millis(), config.drain.as_millis(), config.join_timeout.as_millis(), config.connections, config.payload,
            config.rate, config.queue_messages, config.queue_bytes, config.inflight, self.offered, self.admitted, self.rejected,
            self.schedule_missed, self.would_block, self.inflight_rejected, self.received, self.echoed,
            self.admitted.saturating_sub(self.received), self.duplicates, self.reordered, self.too_old, self.late,
            self.auth_rejected, self.finished_flows, self.pending_messages, self.pending_bytes,
            rtt.sample_count, rtt.p95_us, rtt.p99_us, rtt.p999_us, rtt.max_us, slo,
            config.max_p99_us.map(|value| value.to_string()).unwrap_or_else(|| "unset".into()),
            config.max_unobserved_ppm.map(|value| value.to_string()).unwrap_or_else(|| "unset".into()),
            self.binary_hash, self.package_hash, safe(reason), snapshot);
        std::io::stdout().flush()?;
        Ok(())
    }
}

pub fn safe(text: &str) -> String {
    if text.is_empty() {
        return "none".into();
    }
    text.chars()
        .take(256)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.:=,".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn guard_memory(startup: bool) -> Result<()> {
    let available = fs::read_to_string("/proc/meminfo")?
        .lines()
        .find_map(|line| {
            line.strip_prefix("MemAvailable:")
                .and_then(|value| value.split_whitespace().next())
                .map(str::to_owned)
        })
        .ok_or("MemAvailable unavailable")?
        .parse::<u64>()?;
    let minimum = if startup { 19 } else { 15 } * 1024 * 1024;
    if available < minimum {
        return Err(
            format!("probe_memory_guard available_kib={available} minimum_kib={minimum}").into(),
        );
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    let mut source = fs::File::open(path)?;
    let mut hash = Context::new(&SHA256);
    let mut buffer = [0; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn rss_kib() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        })
}

fn cpu_ticks() -> Option<(u64, u64, u64)> {
    let process = fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<_> = process.rsplit_once(") ")?.1.split_whitespace().collect();
    let ticks = fields
        .get(11)?
        .parse::<u64>()
        .ok()?
        .checked_add(fields.get(12)?.parse::<u64>().ok()?)?;
    let host = fs::read_to_string("/proc/stat").ok()?;
    // guest/guest_nice are already included in user/nice; do not double-count them.
    let total = host
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?
        .into_iter()
        .sum();
    let cpus = host
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu")
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count() as u64;
    Some((ticks, total, cpus))
}
