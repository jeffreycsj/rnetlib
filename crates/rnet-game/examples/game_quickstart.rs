//! Minimal authenticated game echo showing the complete listen/connect/poll/send lifecycle.

use rnet_core::Transport;
use rnet_game::{
    GameClientConfig, GameEvent, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig,
};
use rnet_security::Keypair;
use rnet_transport::ClientSecurity;
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let server_key = Keypair::generate()?;
    let client_key = Keypair::generate()?;
    let server_public_key = server_key.public.clone();
    let protocol = GameProtocol::new(0x4741_4d45, 1);

    // Production normally runs these in different processes. Keeping both runtimes here makes
    // the complete handshake and message path executable as one small example.
    let server = GameRuntime::new(GameRuntimeConfig::production())?;
    let client = GameRuntime::new_with_client_security(
        GameRuntimeConfig::production(),
        ClientSecurity::pinned(client_key, server_public_key),
    )?;
    let listener = server.listen(GameServerConfig {
        transport: Transport::Kcp,
        bind_addr: "127.0.0.1:0".parse()?,
        local_key: server_key,
        initial_encryption: true,
        protocol,
    })?;
    let client_endpoint = client.connect(GameClientConfig {
        transport: Transport::Kcp,
        bind_addr: None,
        remote_addr: server.endpoint_local_addr(listener)?,
        join_ticket: b"validate-me-in-your-login-service".to_vec(),
        protocol,
    })?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sent = false;
    let mut echoed = false;
    while !echoed && Instant::now() < deadline {
        for event in server.poll(64, Duration::from_millis(10)) {
            match event {
                GameEvent::AuthRequest {
                    session,
                    join_ticket,
                    ..
                } => {
                    // Real servers validate the opaque ticket and client identity here.
                    server.auth_decide(
                        session,
                        join_ticket.as_bytes() == b"validate-me-in-your-login-service",
                    )?;
                }
                GameEvent::Message(message) => {
                    // Decode message.payload with protobuf/FlatBuffers in real game code.
                    server.send(message.session, &message.payload)?;
                }
                GameEvent::SessionClosed {
                    session, reason, ..
                } => eprintln!("server session {session} closed: {reason:?}"),
                _ => {}
            }
        }

        for event in client.poll(64, Duration::from_millis(10)) {
            match event {
                GameEvent::SessionReady { endpoint, session } if endpoint == client_endpoint => {
                    // Sending is valid only after this runtime publishes SessionReady.
                    client.send(session, b"opaque protobuf bytes")?;
                    sent = true;
                }
                GameEvent::Message(message) => {
                    println!("echo: {}", String::from_utf8_lossy(&message.payload));
                    echoed = true;
                }
                GameEvent::SessionClosed {
                    session, reason, ..
                } => eprintln!("client session {session} closed: {reason:?}"),
                _ => {}
            }
        }
    }

    if !sent || !echoed {
        return Err("game echo did not complete before timeout".into());
    }
    client.stop(Duration::from_millis(100))?;
    server.stop(Duration::from_millis(100))?;
    Ok(())
}
