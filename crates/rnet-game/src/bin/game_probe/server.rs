//! Server-only runtime. Admission and echo retry state are bounded independently.

use super::{
    config::{Config, Mode, PROTOCOL},
    credentials::{self, Identity},
    queue::EchoQueue,
    report::{guard_memory, Stats},
    wire::{Kind, Packet, Window},
    Result,
};
use rnet_core::Transport;
use rnet_game::{GameEvent, GameRuntime, GameServerConfig};
use rnet_security::Keypair;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::{Duration, Instant};

struct Flow {
    identity: Identity,
    window: Window,
    finish: Option<u64>,
}

pub fn run(config: &Config, runtime: &GameRuntime, token: &[u8], stats: &mut Stats) -> Result<()> {
    let Mode::Server { bind, key } = &config.mode else {
        return Err("server mode required".into());
    };
    let listener = runtime.listen(GameServerConfig {
        transport: config.transport,
        bind_addr: *bind,
        local_key: Keypair::from_private(&credentials::read32(key, true)?)?,
        initial_encryption: true,
        protocol: PROTOCOL,
    })?;
    println!(
        "status=listening role=server transport={} run_id={} address={}",
        config.transport_name(),
        config.run,
        runtime.endpoint_local_addr(listener)?
    );
    std::io::stdout().flush()?;
    let started = Instant::now();
    let deadline = started + config.duration;
    let mut report_at = started + config.report_interval;
    let mut memory_at = started;
    let mut flows = HashMap::<u64, Flow>::new();
    let mut identities = HashSet::new();
    let mut pending = EchoQueue::new(config.queue_messages, config.queue_bytes);
    loop {
        let now = Instant::now();
        if now >= memory_at {
            guard_memory(false)?;
            memory_at = now + Duration::from_millis(100);
        }
        let forwarded = pending.flush(|session, bytes| runtime.send(session, bytes));
        stats.echoed += forwarded.data;
        stats.would_block += forwarded.blocked;
        stats.pending_messages = pending.messages;
        stats.pending_bytes = pending.bytes;
        if let Some(error) = forwarded.error {
            return Err(error.into());
        }
        for event in runtime.poll(256, Duration::from_millis(1)) {
            match event {
                GameEvent::AuthRequest {
                    session,
                    join_ticket,
                    ..
                } => {
                    let identity =
                        credentials::authorize(token, join_ticket.as_bytes(), config.run)
                            .ok()
                            .filter(|identity| {
                                identities.len() < config.connections
                                    && !identities.contains(identity)
                            });
                    runtime.auth_decide(session, identity.is_some())?;
                    if let Some(identity) = identity {
                        identities.insert(identity);
                        flows.insert(
                            session,
                            Flow {
                                identity,
                                window: Window::default(),
                                finish: None,
                            },
                        );
                    } else {
                        stats.auth_rejected += 1;
                    }
                }
                GameEvent::Message(message) => {
                    let flow = flows
                        .get_mut(&message.session)
                        .ok_or("message from unauthenticated probe flow")?;
                    let packet = Packet::decode(&message.payload, config.payload)?;
                    if packet.identity != flow.identity {
                        return Err("probe flow identity mismatch".into());
                    }
                    match packet.kind {
                        Kind::Data => {
                            if flow.finish.is_some_and(|count| {
                                config.transport != Transport::Udp || packet.sequence >= count
                            }) {
                                return Err("probe data outside final admitted count".into());
                            }
                            if stats.disposition(
                                flow.window.observe(packet.sequence, config.transport)?,
                            ) {
                                pending.push(message.session, message.payload, Kind::Data)?;
                                stats.pending_messages = pending.messages;
                                stats.pending_bytes = pending.bytes;
                            }
                        }
                        Kind::Finish => {
                            if flow.finish.is_some_and(|count| count != packet.sequence)
                                || flow.window.next > packet.sequence
                                || (config.transport != Transport::Udp
                                    && flow.window.next != packet.sequence)
                            {
                                return Err("probe finish count mismatch".into());
                            }
                            if flow.finish.replace(packet.sequence).is_none() {
                                stats.finished_flows += 1;
                            }
                            let ack = Packet {
                                kind: Kind::Ack,
                                ..packet
                            }
                            .encode(config.payload);
                            pending.push(message.session, ack.into(), Kind::Ack)?;
                            stats.pending_messages = pending.messages;
                            stats.pending_bytes = pending.bytes;
                        }
                        Kind::Ack => return Err("server received a client ACK".into()),
                    }
                }
                GameEvent::SessionClosed {
                    session, reason, ..
                } => {
                    if let Some(flow) = flows.remove(&session) {
                        let lost_data = pending.remove(session);
                        stats.pending_messages = pending.messages;
                        stats.pending_bytes = pending.bytes;
                        if flow.finish.is_none() || lost_data {
                            return Err(format!(
                                "server_session_closed_before_finish reason={reason:?}"
                            )
                            .into());
                        }
                    }
                }
                GameEvent::ProtocolViolation { session, .. } if flows.contains_key(&session) => {
                    return Err("server_protocol_violation".into())
                }
                GameEvent::EndpointError { endpoint, status }
                    if runtime.endpoint_local_addr(endpoint).is_err() =>
                {
                    return Err(format!("server_endpoint_failed reason={status:?}").into())
                }
                GameEvent::RuntimeStopped => return Err("server_runtime_stopped".into()),
                _ => {}
            }
        }
        stats.pending_messages = pending.messages;
        stats.pending_bytes = pending.bytes;
        if now >= report_at {
            stats.emit(config, &stats.snapshot(runtime), "running", "")?;
            report_at = now + config.report_interval;
        }
        if now >= deadline {
            if stats.received == 0 {
                return Err("server_no_probe_traffic".into());
            }
            if flows.values().any(|flow| flow.finish.is_none()) {
                return Err("server_deadline_before_client_finish".into());
            }
            if pending.messages == 0 && runtime.scheduled_queue_snapshot().queued_messages == 0 {
                break;
            }
            if now >= deadline + config.drain {
                return Err("server_echo_drain_incomplete".into());
            }
        }
    }
    Ok(())
}
