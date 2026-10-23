//! Independent, authenticated server/client probe. Not a production capacity certificate.

#[path = "game_probe/client.rs"]
mod client;
#[path = "game_probe/config.rs"]
mod config;
#[path = "game_probe/credentials.rs"]
mod credentials;
#[path = "game_probe/queue.rs"]
mod queue;
#[path = "game_probe/report.rs"]
mod report;
#[path = "game_probe/server.rs"]
mod server;
#[cfg(test)]
#[path = "game_probe/tests.rs"]
mod tests;
#[path = "game_probe/wire.rs"]
mod wire;

use config::{Config, Mode};
use rnet_game::GameRuntime;
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run() -> Result<()> {
    if std::env::args().skip(1).eq(["--help"]) {
        println!("{}", include_str!("game_probe/usage.txt"));
        return Ok(());
    }
    let config = Config::parse(std::env::args().skip(1))?;
    if let Mode::Keygen { directory } = &config.mode {
        credentials::generate(directory)?;
        println!("status=generated credentials=files_only");
        return Ok(());
    }
    report::guard_memory(true)?;
    let token = credentials::read32(&config.token_file, true)?;
    let runtime = match &config.mode {
        Mode::Client { pin, .. } => GameRuntime::new_with_client_security(
            config.runtime_config(),
            ClientSecurity::pinned(
                Keypair::generate()?,
                credentials::read32(pin, false)?.to_vec(),
            ),
        )?,
        Mode::Server { .. } => GameRuntime::new(config.runtime_config())?,
        Mode::Keygen { .. } => unreachable!(),
    };
    let mut stats = report::Stats::new()?;
    let result = match &config.mode {
        Mode::Server { .. } => server::run(&config, &runtime, &token, &mut stats),
        Mode::Client { .. } => client::run(&config, &runtime, &token, &mut stats),
        Mode::Keygen { .. } => unreachable!(),
    };
    // Capture queue/close evidence before stop clears game state, but never announce success
    // until shutdown has also succeeded. No logger callback calls this lifecycle method.
    let snapshot = stats.snapshot(&runtime);
    let result = result.and_then(|()| stats.check_slo(&config));
    let stopped = runtime.stop(Duration::ZERO);
    let result = result.and_then(|()| stopped.map_err(Into::into));
    stats.emit(
        &config,
        &snapshot,
        if result.is_ok() {
            "completed"
        } else {
            "failed"
        },
        &result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default(),
    )?;
    result
}

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "status=failed phase=probe reason={}",
            report::safe(&error.to_string())
        );
        std::process::exit(1);
    }
}
