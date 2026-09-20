# RNet getting started

## One endpoint API, three transports

Servers call `listen`; clients call `connect`. Set `transport` to TCP, UDP, or KCP in the configuration. The selected transport is immutable for the endpoint and session. `send` therefore needs only a session, message type, and payload.

Every unified connection performs an application handshake before `SessionOpened`. Client identity and server trust belong to the runtime. The client never chooses whether a connection is plaintext or encrypted. The server selects the initial business-data mode and may later switch it; cryptographic control traffic remains authenticated in both modes.

Use TCP for reliable ordered byte-stream delivery, UDP when loss and reordering are acceptable, and KCP when reliable ordered delivery over UDP is required.

## Rust

```rust
use rnet_core::{EventType, Transport};
use rnet_security::Keypair;
use rnet_transport::{
    ClientSecurity, HostClientConfig, NetworkRuntime, RuntimeConfig, SecurityChange,
    SecurityMode, ServerConfig,
};

let server_key = Keypair::generate()?;
let client_key = Keypair::generate()?;
let trust = ClientSecurity::pinned(client_key, server_key.public.clone());
let runtime = NetworkRuntime::new_with_client_security(
    RuntimeConfig::production(), Some(trust),
)?;

let listener = runtime.listen(ServerConfig {
    transport: Transport::Tcp,
    bind_addr: "127.0.0.1:0".parse()?,
    local_key: server_key,
    initial_security: SecurityMode::Encrypted,
})?;
let port = runtime.endpoint_local_addr(listener)?.port();
let client = runtime.connect_host(HostClientConfig {
    transport: Transport::Tcp,
    host: "localhost".to_owned(), // numeric IPv4/IPv6 and DNS names are accepted
    port,
    join_payload: b"login-ticket".to_vec(),
})?;

for event in runtime.poll_events(32, std::time::Duration::from_millis(50)) {
    match event.event_type {
        EventType::AuthRequest => runtime.auth_decide(event.session, true)?,
        EventType::SessionOpened if event.endpoint == client => {
            runtime.send(event.session, 1001, b"hello")?;
        }
        _ => {}
    }
}
```

Only a server-side session may initiate changes:

```rust
runtime.set_security_mode(server_session, SecurityMode::Plaintext)?;
runtime.set_security_mode(server_session, SecurityMode::Encrypted)?;
runtime.rekey_session(server_session)?;
```

Wait for `SecurityChanged` before starting another change. Sends accepted during a transition remain ordered and are emitted after its barrier completes.
Decode the event with `SecurityChange::from_event(&event)` to obtain the typed operation,
active mode, and committed epoch.

## C

```c
rnet_config_v5_t config;
rnet_config_v5_init(&config);

rnet_client_security_t trust = {0};
trust.struct_size = sizeof(trust);
trust.abi_version = RNET_ABI_VERSION;
trust.local_private_key = (rnet_slice_t){client_private, 32};
trust.expected_server_public_key = (rnet_slice_t){server_public, 32};

rnet_runtime_t runtime = 0;
rnet_runtime_create_v5(&config, &trust, &runtime);

rnet_server_config_v2_t server = {0};
server.struct_size = sizeof(server);
server.abi_version = RNET_ABI_VERSION;
server.transport = RNET_TRANSPORT_UDP;
server.initial_security = RNET_SECURITY_ENCRYPTED;
server.bind_host = (rnet_slice_t){(const uint8_t *)"127.0.0.1", 9};
server.bind_port = 7000;
server.local_private_key = (rnet_slice_t){server_private, 32};
rnet_server_open_v2(runtime, &server, &listener);

rnet_client_config_v2_t client = {0};
client.struct_size = sizeof(client);
client.abi_version = RNET_ABI_VERSION;
client.transport = RNET_TRANSPORT_UDP;
client.remote_host = (rnet_slice_t){(const uint8_t *)"game.example.com", 16};
client.remote_port = 7000;
client.join_payload = (rnet_slice_t){ticket, ticket_len};
rnet_client_connect_v2(runtime, &client, &client_endpoint);
```

Alternatively set `trust.verify_server` to a thread-safe callback. Its function and `user_data` must remain valid until `rnet_runtime_destroy` returns. Poll `RNET_EVENT_AUTH_REQUEST`, validate `client_public_key[32] || join_payload`, and call `rnet_session_auth_decide`. Release every nonzero event `buffer_token` exactly once.

Use `rnet_session_send`, `rnet_session_security_set`, and `rnet_session_rekey`. Stop the runtime, release all event buffers, then destroy it.
If a synchronous call fails, copy `rnet_last_error_message()` before the next RNet call for a
human-readable diagnostic.

## C++11

```cpp
rnet::ServerOptions server;
server.transport = rnet::Transport::Kcp;
server.initial_security = rnet::SecurityMode::Encrypted;

std::array<uint8_t, 32> expected;
std::copy(server.keypair.public_key(), server.keypair.public_key() + 32,
          expected.begin());
rnet::Keypair client_key;
rnet_config_v5_t config = {};
rnet::check(rnet_config_v5_init(&config));
rnet::Runtime runtime(config, client_key, expected);

rnet_endpoint_t listener = runtime.listen(server);
rnet::ClientOptions client;
client.transport = rnet::Transport::Kcp;
client.port = runtime.local_port(listener);
client.join_payload = "login-ticket";
rnet_endpoint_t endpoint = runtime.connect(client);
```

Poll events, call `auth_decide`, then use `send`. Server-side sessions can call `set_security(session, mode)` and `rekey(session)`. The wrapper is C++11-only and owns the runtime through RAII.

## Go

```go
serverKey, _ := rnet.GenerateKeypair()
clientKey, _ := rnet.GenerateKeypair()
cfg := rnet.DefaultConfig()
cfg.ClientSecurity = &rnet.ClientSecurity{
    LocalKey: clientKey,
    ExpectedServerPublicKey: serverKey.Public[:],
}
runtime, _ := rnet.NewRuntime(cfg)
defer runtime.Close()

listener, _ := runtime.Listen(rnet.ServerConfig{
    Transport: rnet.TransportTCP,
    Host: "127.0.0.1",
    Keypair: serverKey,
    InitialSecurity: rnet.SecurityEncrypted,
})
port, _ := runtime.LocalPort(listener)
client, _ := runtime.Connect(rnet.ClientConfig{
    Transport: rnet.TransportTCP,
    Host: "127.0.0.1", Port: port,
    JoinPayload: []byte("login-ticket"),
})
_ = client
```

Handle `EventAuthRequest` with `AuthDecide`, then call `Send`. Use `SetSecurity` and `Rekey` only with a server session.

## Production sizing and load probe

Rust deployments can tune `max_sessions_per_endpoint`, `handshake_timeout`,
`datagram_idle_timeout`, event capacity, write capacity, payload limits, worker threads, and KCP
MTU through `RuntimeConfig`. TCP defaults to `TCP_NODELAY`; optional send/receive buffer requests are
available in Rust `RuntimeConfig`, C/C++ `rnet_config_v5_t`, and Go `Config`. Runtime-wide endpoint and pending-handshake limits complement per-listener capacity so unauthenticated traffic cannot
bypass admission control. Size queues from an explicit memory budget; increasing every limit is not
a substitute for backpressure in the application.

Host names and textual IPv4/IPv6 addresses are supported. TCP, UDP, and KCP try resolved candidates
under a single deadline. UDP/KCP automatically rebind their client socket when retrying a candidate
from another address family.

Run the encrypted local probe before deployment, then repeat across real hosts with representative
loss, RTT, payloads, connection counts, and application event processing:

```sh
cargo run --release -p rnet-transport --example load_probe -- tcp 1000000 4096
cargo run --release -p rnet-transport --example load_probe -- kcp 100000 4096
cargo run --release -p rnet-transport --example load_probe -- udp 100000 1024
./scripts/run-soak.sh tcp 86400 4096
```

The probe reports CPU, RSS, message rate, MiB/s, queue/error counters, and P50/P90/P95/P99/P99.9/max latency samples. Enterprise readiness is
an environment-specific capacity result: define an SLO, run soak and fault tests at or above peak
load, and alert on `events_dropped`, `send_would_block`, protocol errors, queue percentiles, and KCP
update delay.

## Observability and shutdown

Latency snapshots expose sample count, P50, P90, P95, P99, P99.9, and max in microseconds for connect, cryptographic handshake, authorization wait, send queue, event queue, KCP RTT/update delay, and logger callback latency. Go `LatencyMetrics` reads cumulative values and `DrainLatencyMetrics` rotates the interval; C++ passes `true` to `latency_metrics`. Periodic log summaries are enabled with `rnet_metrics_log_interval_set` or its SDK wrapper. Go can set `Config.Logger` to receive `LogRecord`; callbacks must be thread-safe and return quickly.

Close sessions/endpoints when useful, stop the runtime with a drain timeout, release borrowed C event buffers, then destroy/close the runtime. `WOULD_BLOCK` means a bounded queue or transition slot is full; poll progress and retry according to application backpressure policy.
