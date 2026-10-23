//! Strict CLI configuration; queue limits are independent layers, not an RSS promise.

use super::Result;
use rnet_core::Transport;
use rnet_game::{GameProtocol, GameRuntimeConfig};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub(super) const PROTOCOL: GameProtocol = GameProtocol::new(0x524e_5052_4f42_4501, 1, 1);

pub(super) enum Mode {
    Keygen {
        directory: PathBuf,
    },
    Server {
        bind: SocketAddr,
        key: PathBuf,
    },
    Client {
        host: String,
        port: u16,
        pin: PathBuf,
        generator: u64,
    },
}

pub(super) struct Config {
    /// Selects exactly one role; no hidden loopback runtime is created.
    pub mode: Mode,
    /// Fixed for every endpoint in this run; both peers must select the same transport.
    pub transport: Transport,
    /// Nonzero benchmark identity, carried inside authenticated application traffic.
    pub run: u64,
    /// Client connection count, or server cap on distinct flows over the entire run.
    pub connections: usize,
    /// Total application data bytes, including the probe's 32-byte identity/header.
    pub payload: usize,
    /// Server lifetime from listen, or client load duration after all joins complete.
    pub duration: Duration,
    /// Echo grace period; the client reserves another equal period for finish ACKs.
    pub drain: Duration,
    /// Total client join deadline, also used for individual transport handshakes.
    pub join_timeout: Duration,
    /// Wall interval between cumulative snapshots, not a histogram reset period.
    pub report_interval: Duration,
    /// Offered probes per client stream per second, not guaranteed admitted throughput.
    pub rate: u64,
    /// Independent application retry and game scheduler limits, not one shared budget.
    pub queue_messages: usize,
    /// Per-layer queued payload byte cap; metadata and other layers consume extra memory.
    pub queue_bytes: usize,
    /// Total outstanding client timestamps, shared across streams.
    pub inflight: usize,
    /// Restricted file containing the 32-byte shared join-authentication secret.
    pub token_file: PathBuf,
    /// Optional cumulative client RTT threshold in microseconds; no samples cannot pass.
    pub max_p99_us: Option<u64>,
    /// Optional round-trip unobserved-echo threshold in parts per million, not link loss.
    pub max_unobserved_ppm: Option<u64>,
}

fn required(values: &mut BTreeMap<String, String>, name: &str) -> Result<String> {
    values
        .remove(name)
        .ok_or_else(|| format!("missing {name}").into())
}

fn number(
    values: &mut BTreeMap<String, String>,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64> {
    let value = values
        .remove(name)
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| format!("invalid integer for {name}"))?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("out of bounds: {name}").into());
    }
    Ok(value)
}

impl Config {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut args = args;
        let mode = args
            .next()
            .ok_or("usage: game_probe keygen|server|client [--name value ...]")?;
        let mut values = BTreeMap::new();
        while let Some(name) = args.next() {
            if !name.starts_with("--") {
                return Err("expected --name value".into());
            }
            let value = args.next().ok_or("option is missing a value")?;
            if values.insert(name, value).is_some() {
                return Err("duplicate option".into());
            }
        }
        let selected = match mode.as_str() {
            "keygen" => Mode::Keygen {
                directory: required(&mut values, "--dir")?.into(),
            },
            "server" => Mode::Server {
                bind: required(&mut values, "--bind")?.parse()?,
                key: required(&mut values, "--key-file")?.into(),
            },
            "client" => Mode::Client {
                host: required(&mut values, "--host")?,
                port: required(&mut values, "--port")?.parse::<u16>()?,
                pin: required(&mut values, "--pin-file")?.into(),
                generator: required(&mut values, "--generator-id")?.parse::<u64>()?,
            },
            _ => return Err("mode must be keygen, server or client".into()),
        };
        let keygen = matches!(selected, Mode::Keygen { .. });
        if keygen && !values.is_empty() {
            return Err("keygen accepts only --dir".into());
        }
        let transport = if keygen {
            Transport::Tcp
        } else {
            match required(&mut values, "--transport")?.as_str() {
                "tcp" => Transport::Tcp,
                "udp" => Transport::Udp,
                "kcp" => Transport::Kcp,
                _ => return Err("transport must be tcp, udp or kcp".into()),
            }
        };
        let run = if keygen {
            1
        } else {
            required(&mut values, "--run-id")?.parse::<u64>()?
        };
        let token_file = if keygen {
            PathBuf::new()
        } else {
            required(&mut values, "--token-file")?.into()
        };
        if let Mode::Client {
            host,
            port,
            generator,
            ..
        } = &selected
        {
            if host.is_empty()
                || host.chars().any(char::is_whitespace)
                || host.contains('/')
                || *port == 0
                || *generator == 0
            {
                return Err("invalid client host, port or generator ID".into());
            }
        }
        if run == 0 {
            return Err("run ID must be nonzero".into());
        }
        let config = Self {
            mode: selected,
            transport,
            run,
            token_file,
            connections: number(&mut values, "--connections", 1, 1, 4096)? as usize,
            payload: number(
                &mut values,
                "--payload-bytes",
                64,
                32,
                if transport == Transport::Udp {
                    1024
                } else {
                    65536
                },
            )? as usize,
            duration: Duration::from_millis(number(
                &mut values,
                "--duration-ms",
                10000,
                1,
                259_200_000,
            )?),
            drain: Duration::from_millis(number(&mut values, "--drain-ms", 5000, 100, 60000)?),
            join_timeout: Duration::from_millis(number(
                &mut values,
                "--join-timeout-ms",
                10000,
                100,
                60000,
            )?),
            report_interval: Duration::from_millis(number(
                &mut values,
                "--report-ms",
                1000,
                100,
                60000,
            )?),
            rate: number(&mut values, "--rate", 20, 1, 10000)?,
            queue_messages: number(&mut values, "--queue-messages", 4096, 1, 65536)? as usize,
            queue_bytes: number(
                &mut values,
                "--queue-bytes",
                8 * 1024 * 1024,
                256,
                64 * 1024 * 1024,
            )? as usize,
            inflight: number(&mut values, "--inflight", 4096, 1, 65536)? as usize,
            max_p99_us: if values.contains_key("--max-rtt-p99-us") {
                Some(number(&mut values, "--max-rtt-p99-us", 1, 1, 60_000_000)?)
            } else {
                None
            },
            max_unobserved_ppm: if values.contains_key("--max-unobserved-ppm") {
                Some(number(
                    &mut values,
                    "--max-unobserved-ppm",
                    0,
                    0,
                    1_000_000,
                )?)
            } else {
                None
            },
        };
        if !values.is_empty() {
            return Err("unknown or inapplicable option".into());
        }
        if config.queue_bytes < config.payload + 256 {
            return Err("queue bytes must cover payload plus framing allowance".into());
        }
        if !matches!(config.mode, Mode::Client { .. })
            && (config.max_p99_us.is_some() || config.max_unobserved_ppm.is_some())
        {
            return Err("echo SLO options apply only to clients".into());
        }
        Ok(config)
    }

    pub fn role(&self) -> &'static str {
        match self.mode {
            Mode::Server { .. } => "server",
            Mode::Client { .. } => "client",
            Mode::Keygen { .. } => "keygen",
        }
    }

    pub fn transport_name(&self) -> &'static str {
        match self.transport {
            Transport::Tcp => "tcp",
            Transport::Udp => "udp",
            Transport::Kcp => "kcp",
        }
    }

    pub fn runtime_config(&self) -> GameRuntimeConfig {
        let mut config = GameRuntimeConfig::production();
        config.network.worker_threads = 2;
        config.network.max_endpoints = self.connections + 1;
        config.network.max_sessions_per_endpoint = self.connections;
        config.network.max_sessions_per_ip = self.connections;
        config.network.max_pending_handshakes = self.connections;
        config.network.handshake_burst_per_ip = self.connections as u32;
        config.network.handshake_rate_per_ip = self.connections as u32;
        config.network.connect_timeout = self.join_timeout;
        config.network.handshake_timeout = self.join_timeout;
        config.network.dns_timeout = self.join_timeout.min(Duration::from_secs(1));
        config.network.event_queue_capacity = (self.connections * 8).max(256);
        config.network.max_event_bytes = self.queue_bytes;
        config.network.write_queue_capacity = self.queue_messages.min(1024);
        config.network.max_runtime_queued_bytes = self.queue_bytes;
        config.network.max_session_queued_bytes = self.queue_bytes;
        config.scheduled_queue.max_queued_bytes = self.queue_bytes;
        config.scheduled_queue.max_session_queued_bytes = self.queue_bytes;
        config.scheduled_queue.max_queued_messages = self.queue_messages;
        config
    }
}
