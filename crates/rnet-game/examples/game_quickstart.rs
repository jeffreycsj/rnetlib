//! Authenticated game echo with bounded application retries and explicit shutdown.

use rnet_core::{ErrorCode, Result, RnetError, Transport};
use rnet_game::{
    BoundedLogger, GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig,
    GameServerConfig, LoggerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const TICKET: &[u8] = b"demo-only-login-ticket";
const PAYLOADS: [&[u8]; 2] = [b"binary\0game\xff", b""];

struct Options {
    transport: Transport, // Fixed at listen/connect, never supplied to send.
    accept: bool,         // Demo denial switch, not a real authorization policy.
    single_slot: bool,    // Forces real bounded game admission to demonstrate retries.
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let transport = match args.next().as_deref() {
            None | Some("kcp") => Transport::Kcp,
            Some("tcp") => Transport::Tcp,
            Some("udp") => Transport::Udp,
            _ => return Err(usage()),
        };
        let mut options = Self {
            transport,
            accept: true,
            single_slot: false,
        };
        for flag in args {
            match flag.as_str() {
                "--deny" if options.accept => options.accept = false,
                "--one-slot" if !options.single_slot => options.single_slot = true,
                _ => return Err(usage()),
            }
        }
        Ok(options)
    }
}

fn usage() -> RnetError {
    RnetError::new(
        ErrorCode::InvalidArgument,
        "usage: game_quickstart [tcp|udp|kcp] [--deny] [--one-slot]",
    )
}

fn main() -> Result<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let report = run(&options, Duration::from_secs(10))?;
    println!("{:?} echo complete: {} binary/empty messages, {} blocked iterations; both runtimes stopped",
        options.transport, report.received, report.blocked);
    Ok(())
}

#[derive(Debug)]
struct Report {
    received: usize, // Peer echoes verified byte-for-byte, not local send admissions.
    blocked: usize,  // Iterations encountering game-stage WouldBlock, not packet loss.
}

fn logger() -> Result<BoundedLogger> {
    // stderr is for this short demo. Production supplies a fast sink and monitors
    // dropped logs. Never stop the runtime from its callback or log credential bytes.
    BoundedLogger::new(LoggerConfig::default(), |record| {
        eprintln!(
            "{} runtime={} endpoint={} session={} code={:?} {}",
            record.event_name,
            record.runtime,
            record.endpoint,
            record.session,
            record.error_code,
            record.message
        );
    })
}

fn run(options: &Options, timeout: Duration) -> Result<Report> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .filter(|_| !timeout.is_zero())
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidArgument, "invalid demo deadline"))?;
    // Production loads a persistent server identity and distributes its pin through
    // a trusted channel. Never trust a key learned over an unauthenticated socket.
    let server_key = Keypair::generate()
        .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
    let client_key = Keypair::generate()
        .map_err(|error| RnetError::new(ErrorCode::CryptoError, error.to_string()))?;
    let server_public_key = server_key.public.clone();
    let mut config = GameRuntimeConfig::production();
    if options.single_slot {
        config.scheduled_queue.max_queued_messages = 1;
    }
    let server_logger = logger()?;
    let client_logger = logger()?;
    // These runtimes normally live in different processes. Configure both loggers
    // before creating either runtime so a logger setup error leaves no live endpoint.
    let server = GameRuntime::new(config.clone())?.with_logger(server_logger);
    let client = match GameRuntime::new_with_client_security(
        config,
        ClientSecurity::pinned(client_key, server_public_key),
    ) {
        Ok(runtime) => runtime.with_logger(client_logger),
        Err(error) => return finish(Err(error), Ok(()), stop_runtime(&server)),
    };
    let result = exchange(&server, &client, server_key, options, deadline);
    // Evaluate both stops before propagating an error. Exchange errors retain their
    // original category; cleanup failures are also reported and never hidden by success.
    finish(result, stop_runtime(&client), stop_runtime(&server))
}

fn exchange(
    server: &GameRuntime,
    client: &GameRuntime,
    server_key: Keypair,
    options: &Options,
    deadline: Instant,
) -> Result<Report> {
    let protocol = GameProtocol::new(0x4741_4d45, 1, 1);
    let listener = server.listen(GameServerConfig {
        transport: options.transport,
        bind_addr: ([127, 0, 0, 1], 0).into(),
        local_key: server_key,
        initial_encryption: true,
        protocol,
    })?;
    let client_endpoint = client.connect(GameClientConfig {
        transport: options.transport,
        bind_addr: None,
        remote_addr: server.endpoint_local_addr(listener)?,
        join_ticket: TICKET.to_vec(),
        protocol,
    })?;

    let mut server_session = None;
    let mut client_session = None;
    let mut server_pending = PendingSends::default();
    let mut client_pending = PendingSends::default();
    let mut report = Report {
        received: 0,
        blocked: 0,
    };
    while report.received < PAYLOADS.len() && Instant::now() < deadline {
        for event in server.poll(64, Duration::from_millis(1)) {
            check_event(&event)?;
            match event {
                GameEvent::AuthRequest {
                    session,
                    join_ticket,
                    ..
                } => {
                    // Demo only: real authorization validates ticket signature, expiry,
                    // audience and binding to the authenticated client_public_key.
                    server
                        .auth_decide(session, options.accept && join_ticket.as_bytes() == TICKET)?;
                }
                GameEvent::SessionReady { session, endpoint } if endpoint == listener => {
                    server_session = Some(session)
                }
                GameEvent::Message(message) => {
                    if Some(message.session) != server_session {
                        return Err(RnetError::new(
                            ErrorCode::InvalidState,
                            "server message before game readiness",
                        ));
                    }
                    // Copy into bounded application storage, independent of this event batch.
                    server_pending.stage(&message.payload)?;
                }
                _ => {}
            }
        }

        for event in client.poll(64, Duration::from_millis(1)) {
            check_event(&event)?;
            match event {
                GameEvent::SessionReady { endpoint, session } if endpoint == client_endpoint => {
                    client_session = Some(session);
                    for payload in PAYLOADS {
                        client_pending.stage(payload)?;
                    }
                }
                GameEvent::Message(message) => {
                    if Some(message.session) != client_session
                        || PAYLOADS.get(report.received).copied() != Some(message.payload.as_ref())
                    {
                        return Err(RnetError::new(
                            ErrorCode::ProtocolError,
                            "echo payload/order mismatch",
                        ));
                    }
                    report.received += 1;
                }
                _ => {}
            }
        }
        if let Some(session) = server_session {
            report.blocked +=
                usize::from(server_pending.flush(|bytes| server.send(session, bytes), deadline)?);
        }
        if let Some(session) = client_session {
            report.blocked +=
                usize::from(client_pending.flush(|bytes| client.send(session, bytes), deadline)?);
        }
    }
    if report.received != PAYLOADS.len() {
        return Err(RnetError::new(
            ErrorCode::Timeout,
            "echo deadline exceeded; UDP does not guarantee delivery",
        ));
    }
    client.close_session(client_session.expect("verified messages require readiness"))?;
    server.close_endpoint(listener)?;
    Ok(report)
}

fn check_event(event: &GameEvent) -> Result<()> {
    let (code, phase) = match event {
        GameEvent::JoinFailed { reason, .. } => (*reason, "join failed"),
        GameEvent::ProtocolRejected { reason, .. } => (*reason, "protocol rejected"),
        GameEvent::SessionClosed { reason, .. } => {
            (*reason, "session closed before echo completed")
        }
        GameEvent::EndpointError { status, .. } => {
            (*status, "endpoint error; demo aborts even if recoverable")
        }
        GameEvent::ProtocolViolation { .. } => (ErrorCode::ProtocolError, "protocol violation"),
        GameEvent::RuntimeStopped => (ErrorCode::InvalidState, "runtime stopped before completion"),
        _ => return Ok(()),
    };
    Err(RnetError::new(
        if code == ErrorCode::Ok {
            ErrorCode::InvalidState
        } else {
            code
        },
        phase,
    ))
}

fn stop_runtime(runtime: &GameRuntime) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match runtime.stop(Duration::ZERO) {
            Ok(()) => return Ok(()),
            Err(error) if error.code() == ErrorCode::WouldBlock && Instant::now() < deadline => {
                // Shutdown intentionally discards remaining application events, not buffers
                // borrowed by other threads. This demo has exactly one consumer per runtime.
                drop(runtime.poll(64, Duration::from_millis(1)));
            }
            Err(error) => return Err(error),
        }
    }
}

fn finish(
    result: Result<Report>,
    client_stop: Result<()>,
    server_stop: Result<()>,
) -> Result<Report> {
    for error in [&client_stop, &server_stop]
        .into_iter()
        .filter_map(|value| value.as_ref().err())
    {
        eprintln!("shutdown failed: {error}");
    }
    let report = result?;
    client_stop?;
    server_stop?;
    Ok(report)
}

/// Application-owned retry storage, intentionally only two small messages per peer.
#[derive(Default)]
struct PendingSends(VecDeque<Vec<u8>>);

impl PendingSends {
    fn stage(&mut self, payload: &[u8]) -> Result<()> {
        if payload.len() > 128 {
            return Err(RnetError::new(
                ErrorCode::MessageTooLarge,
                "demo payload limit is 128 bytes",
            ));
        }
        if self.0.len() >= 2 {
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "demo retry queue is full",
            ));
        }
        self.0.push_back(payload.to_vec());
        Ok(())
    }

    // true means retry after a positive-capacity poll, not immediately or only on
    // Writable (which describes transport capacity, not this game-stage queue).
    fn flush(
        &mut self,
        mut send: impl FnMut(&[u8]) -> Result<()>,
        deadline: Instant,
    ) -> Result<bool> {
        while let Some(payload) = self.0.front() {
            if Instant::now() >= deadline {
                return Err(RnetError::new(
                    ErrorCode::Timeout,
                    "demo retry deadline exceeded",
                ));
            }
            match send(payload) {
                Ok(()) => {
                    self.0.pop_front();
                }
                Err(error) if error.code() == ErrorCode::WouldBlock => return Ok(true),
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "../tests/support/quickstart_contracts.rs"]
mod tests;
