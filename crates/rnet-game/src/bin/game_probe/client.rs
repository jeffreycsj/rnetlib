//! Client-only load generator; local admission is never counted as a delivered echo.

use super::{
    config::{Config, Mode, PROTOCOL},
    credentials::{self, Identity},
    report::{guard_memory, Stats},
    wire::{Disposition, Kind, Packet, Window},
    Result,
};
use rnet_core::{ErrorCode, Transport};
use rnet_game::{GameEvent, GameHostClientConfig, GameRuntime};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

struct Flow {
    identity: Identity,
    session: Option<u64>,
    offered: u64,
    admitted: u64,
    received: u64,
    window: Window,
    pending: BTreeMap<u64, Instant>,
    finish_sent: bool,
    finish_at: Instant,
    done: bool,
}

pub fn run(config: &Config, runtime: &GameRuntime, token: &[u8], stats: &mut Stats) -> Result<()> {
    let Mode::Client {
        host,
        port,
        generator,
        ..
    } = &config.mode
    else {
        return Err("client mode required".into());
    };
    let started = Instant::now();
    let mut flows = Vec::with_capacity(config.connections);
    let mut endpoints = HashMap::new();
    for index in 0..config.connections {
        if started.elapsed() >= config.join_timeout {
            return Err("client_join_timeout".into());
        }
        let identity = Identity {
            run: config.run,
            generator: *generator,
            stream: index as u32,
        };
        let endpoint = runtime.connect_host(GameHostClientConfig {
            transport: config.transport,
            host: host.clone(),
            port: *port,
            join_ticket: credentials::ticket(token, identity),
            protocol: PROTOCOL,
        })?;
        endpoints.insert(endpoint, index);
        flows.push(Flow {
            identity,
            session: None,
            offered: 0,
            admitted: 0,
            received: 0,
            window: Window::default(),
            pending: BTreeMap::new(),
            finish_sent: false,
            finish_at: started,
            done: false,
        });
    }
    let mut sessions = HashMap::new();
    let mut pending_count = 0;
    while flows.iter().any(|flow| flow.session.is_none()) {
        guard_memory(false)?;
        if started.elapsed() >= config.join_timeout {
            return Err("client_join_timeout".into());
        }
        events(
            config,
            runtime,
            &mut flows,
            &endpoints,
            &mut sessions,
            &mut pending_count,
            stats,
        )?;
    }
    let traffic_start = Instant::now();
    let load_deadline = traffic_start + config.duration;
    let drain_deadline = load_deadline + config.drain;
    let finish_deadline = drain_deadline + config.drain;
    let mut memory_at = traffic_start;
    let mut report_at = traffic_start + config.report_interval;
    let mut rotation = 0;
    loop {
        let now = Instant::now();
        if now >= memory_at {
            guard_memory(false)?;
            memory_at = now + Duration::from_millis(100);
        }
        if config.transport == Transport::Udp {
            for flow in &mut flows {
                while flow
                    .pending
                    .first_key_value()
                    .is_some_and(|(_, sent)| now.duration_since(*sent) >= config.drain)
                {
                    flow.pending.pop_first();
                    pending_count -= 1;
                    stats.pending_messages = pending_count;
                }
            }
        }
        let elapsed = now
            .saturating_duration_since(traffic_start)
            .min(config.duration);
        let scheduled = (elapsed.as_nanos() * u128::from(config.rate) / 1_000_000_000) as u64;
        let mut had_offers = false;
        for offset in 0..flows.len() {
            let index = (rotation + offset) % flows.len();
            let flow = &mut flows[index];
            let due = scheduled - flow.offered;
            if due == 0 {
                continue;
            }
            had_offers = true;
            flow.offered = scheduled;
            stats.offered += due;
            // Missed scheduling ticks are rejected offers, not hidden reductions in target rate.
            stats.rejected += due - 1;
            stats.schedule_missed += due - 1;
            if pending_count >= config.inflight {
                stats.rejected += 1;
                stats.inflight_rejected += 1;
                continue;
            }
            let data = Packet {
                kind: Kind::Data,
                identity: flow.identity,
                sequence: flow.admitted,
            }
            .encode(config.payload);
            let sent = Instant::now();
            match runtime.send(flow.session.expect("all joins completed"), &data) {
                Ok(()) => {
                    flow.pending.insert(flow.admitted, sent);
                    flow.admitted += 1;
                    stats.admitted += 1;
                    pending_count += 1;
                    stats.pending_messages = pending_count;
                }
                Err(error) if error.code() == ErrorCode::WouldBlock => {
                    stats.rejected += 1;
                    stats.would_block += 1;
                }
                Err(error) => {
                    stats.rejected += 1;
                    return Err(error.into());
                }
            }
        }
        if had_offers {
            rotation = (rotation + 1) % flows.len();
        }
        events(
            config,
            runtime,
            &mut flows,
            &endpoints,
            &mut sessions,
            &mut pending_count,
            stats,
        )?;
        stats.pending_messages = pending_count;
        if now >= load_deadline {
            if config.transport != Transport::Udp && now >= drain_deadline && pending_count != 0 {
                return Err("client_reliable_echo_drain_incomplete".into());
            }
            for flow in &mut flows {
                let drained = if config.transport == Transport::Udp {
                    now >= drain_deadline
                } else {
                    flow.pending.is_empty()
                };
                if !drained || flow.done || now < flow.finish_at {
                    continue;
                }
                let finish = Packet {
                    kind: Kind::Finish,
                    identity: flow.identity,
                    sequence: flow.admitted,
                }
                .encode(config.payload);
                match runtime.send(flow.session.expect("joined"), &finish) {
                    Ok(()) => {
                        flow.finish_sent = true;
                        flow.finish_at = now + Duration::from_millis(100);
                    }
                    Err(error) if error.code() == ErrorCode::WouldBlock => {
                        stats.would_block += 1;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            if flows.iter().all(|flow| flow.done) {
                break;
            }
            if now >= finish_deadline {
                return Err("client_finish_ack_timeout".into());
            }
        }
        if now >= report_at {
            stats.emit(config, &stats.snapshot(runtime), "running", "")?;
            report_at = now + config.report_interval;
        }
    }
    if stats.admitted == 0 {
        return Err("client_no_admitted_probes".into());
    }
    for flow in flows {
        println!("status=flow_completed role=client run_id={} generator_id={} stream={} offered={} admitted={} received={}",
            config.run, generator, flow.identity.stream, flow.offered, flow.admitted, flow.received);
    }
    Ok(())
}

fn events(
    config: &Config,
    runtime: &GameRuntime,
    flows: &mut [Flow],
    endpoints: &HashMap<u64, usize>,
    sessions: &mut HashMap<u64, usize>,
    pending_count: &mut usize,
    stats: &mut Stats,
) -> Result<()> {
    let wait = Duration::from_nanos((1_000_000_000 / config.rate).min(1_000_000));
    for event in runtime.poll(256, wait) {
        match event {
            GameEvent::SessionReady { endpoint, session } => {
                let index = *endpoints
                    .get(&endpoint)
                    .ok_or("unexpected ready endpoint")?;
                if flows[index].session.replace(session).is_some() {
                    return Err("duplicate ready session".into());
                }
                sessions.insert(session, index);
            }
            GameEvent::Message(message) => {
                let flow = &mut flows[*sessions
                    .get(&message.session)
                    .ok_or("message before join readiness")?];
                let packet = Packet::decode(&message.payload, config.payload)?;
                if packet.identity != flow.identity {
                    return Err("echo identity mismatch".into());
                }
                match packet.kind {
                    Kind::Data => {
                        if packet.sequence >= flow.admitted {
                            return Err("echo sequence was never admitted".into());
                        }
                        let disposition = flow.window.observe(packet.sequence, config.transport)?;
                        let sent = if disposition != Disposition::Duplicate {
                            flow.pending.remove(&packet.sequence)
                        } else {
                            None
                        };
                        if sent.is_some() {
                            *pending_count -= 1;
                        }
                        stats.pending_messages = *pending_count;
                        if stats.disposition(disposition) {
                            flow.received += 1;
                            if let Some(sent) = sent {
                                stats.rtt.record(sent.elapsed());
                            } else {
                                stats.late += 1;
                            }
                        }
                    }
                    Kind::Ack => {
                        if !flow.finish_sent || packet.sequence != flow.admitted {
                            return Err("unexpected finish acknowledgement".into());
                        }
                        if !flow.done {
                            flow.done = true;
                            stats.finished_flows += 1;
                        }
                    }
                    Kind::Finish => return Err("client received a finish request".into()),
                }
            }
            GameEvent::JoinFailed { reason, .. } => {
                return Err(format!("client_join_failed reason={reason:?}").into())
            }
            GameEvent::SessionClosed {
                session, reason, ..
            } => {
                if !sessions
                    .get(&session)
                    .is_some_and(|index| flows[*index].done)
                {
                    return Err(format!("client_join_or_session_closed reason={reason:?}").into());
                }
            }
            GameEvent::ProtocolRejected { reason, .. } => {
                return Err(format!("client_join_rejected reason={reason:?}").into())
            }
            GameEvent::ProtocolViolation { .. } => return Err("client_protocol_violation".into()),
            GameEvent::EndpointError { endpoint, status }
                if runtime.endpoint_local_addr(endpoint).is_err() =>
            {
                return Err(format!("client_join_endpoint_failed reason={status:?}").into())
            }
            GameEvent::SecurityChanged {
                encrypted: false, ..
            } => return Err("probe requires encrypted business traffic".into()),
            GameEvent::RuntimeStopped => return Err("client_runtime_stopped".into()),
            _ => {}
        }
    }
    Ok(())
}
