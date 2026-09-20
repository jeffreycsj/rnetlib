use rnet_core::{Event, EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientConfig, ClientSecurity, NetworkRuntime, RuntimeConfig, SecurityMode, ServerConfig,
};
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

fn localhost(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn poll_until(
    runtime: &NetworkRuntime,
    timeout: Duration,
    predicate: impl Fn(&Event) -> bool,
) -> Option<Event> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(event) = runtime
            .poll_events(32, Duration::from_millis(20))
            .into_iter()
            .find(&predicate)
        {
            return Some(event);
        }
    }
    None
}

fn main() -> Result<(), Box<dyn Error>> {
    let server_key = Keypair::generate()?;
    let client_key = Keypair::generate()?;
    let trust = ClientSecurity::pinned(client_key, server_key.public.clone());
    let runtime = NetworkRuntime::new_with_client_security(RuntimeConfig::default(), Some(trust))?;

    // Only this enum changes when selecting UDP or KCP; the API names stay the same.
    let transport = Transport::Tcp;
    let listener = runtime.listen(ServerConfig {
        transport,
        bind_addr: localhost(0),
        local_key: server_key,
        initial_security: SecurityMode::Encrypted,
    })?;
    let remote = localhost(runtime.endpoint_local_addr(listener)?.port());
    let client = runtime.connect(ClientConfig {
        transport,
        bind_addr: None,
        remote_addr: remote,
        join_payload: b"example-ticket".to_vec(),
    })?;

    let auth = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::AuthRequest
    })
    .ok_or("authentication request timed out")?;
    runtime.auth_decide(auth.session, true)?;
    let client_open = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::SessionOpened && event.endpoint == client
    })
    .ok_or("client session timed out")?;
    runtime.send(client_open.session, 1001, b"hello")?;

    let message = poll_until(&runtime, Duration::from_secs(2), |event| {
        event.event_type == EventType::Message
    })
    .ok_or("message timed out")?;
    println!("{}", String::from_utf8_lossy(&message.data));
    runtime.stop(Duration::from_millis(100))?;
    Ok(())
}
