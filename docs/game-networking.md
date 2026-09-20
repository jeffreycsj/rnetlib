# Game networking quick start (Rust)

`rnet-game` is the current game-facing API. It is a production candidate, not a completed game SDK: heartbeats, quality snapshots, reconnect tickets, real-time snapshot replacement, and game-level C/C++11/Go bindings remain unfinished. The transport library underneath still exposes its existing C/C++11/Go APIs.

```rust
use rnet_core::Transport;
use rnet_game::{GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig};
use rnet_security::Keypair;

let runtime = GameRuntime::new(GameRuntimeConfig::production())?;
let listener = runtime.listen(GameServerConfig {
    transport: Transport::Kcp, // fixed for this endpoint; no transport argument on send
    bind_addr: "0.0.0.0:7000".parse()?,
    local_key: Keypair::generate()?,
    initial_encryption: true,
    protocol: GameProtocol::new(0x4741_4d45, 1),
})?;
```

For clients, construct `ClientSecurity::pinned(client_key, server_public_key)` and pass it to `GameRuntime::new_with_client_security`. Then call `connect(GameClientConfig { ... })` for a numeric address, or `connect_host(GameHostClientConfig { ... })` for a hostname. Both configs select transport once and carry an opaque `join_ticket`; neither exposes an encryption setting.

Poll `runtime.poll(capacity, timeout)` for `GameEvent::AuthRequest`. The library first rejects mismatched protocol ID/version, then exposes the original `join_ticket` and the client's Noise public key to the application. After validating the ticket, call `runtime.auth_decide(session, true)`. Both ends receive `GameEvent::SessionReady` only after authorization.

```rust
runtime.send(session, protobuf_bytes)?;
// GameEvent::Message(message).payload contains the original protobuf bytes.
```

The application defines its packet type in protobuf, FlatBuffers, or another payload schema. The game API has no `msg_type`, `stream_id`, or per-send TCP/UDP/KCP parameter. Optional network-owned sequence and simulation tick metadata use `send_with_options`; they do not determine the business packet type. Raw UDP remains unreliable; use KCP or TCP when the application needs reliable delivery.

For operations, `metrics_snapshot`, `prometheus_snapshot`, `latency_snapshot`, and `drain_latency_window` expose the transport's bounded resource gauges and latency percentiles. `close_session`, `close_endpoint`, and `stop` are available on `GameRuntime`; game code does not need to hold a second, lower-level runtime object. Game-specific heartbeat, jitter, and tick-delay metrics have not been implemented yet.

## Server-controlled encryption

With `GameRuntimeConfig::production()`, business plaintext is disabled. A deployment that explicitly accepts the risk can use `GameRuntimeConfig::production().allow_plaintext_business_data(true)` while retaining the production policy that blocks unauthenticated legacy endpoints. The server can then call `runtime.set_encryption(session, false)` and later `runtime.set_encryption(session, true)`; clients follow authenticated security-control messages automatically. Plaintext business records genuinely lack confidentiality and integrity. Authentication and mode-switch control stay protected by Noise.

The server may call `runtime.rekey(session)` to rotate established session keys without changing whether business data is encrypted. Clients cannot initiate a rekey. `SecurityChanged` reports the completed operation and security epoch; the application continues using the same session handle and `send` API.

The exact-version join gate is not a downgrade negotiation: client and server must configure the same nonzero protocol ID and version. `build_id` and `capabilities` are authenticated metadata for application authorization, not permission to activate network-library features. Future version-range negotiation requires a server-authenticated selected-version response and is not implemented yet.

The complete live-session test in [game_runtime.rs](../crates/rnet-game/tests/game_runtime.rs) covers TCP, UDP, KCP, plaintext-to-encrypted-to-plaintext transitions, rekey, hostname joining, and version rejection.

## Current production limits

The KCP adapter rejects malformed values near the pinned dependency's signed sequence boundary, but automatic rotation of an exceptionally long-lived KCP conversation is not implemented. Deployments approaching two billion packets on one KCP session must re-establish that session before the boundary. This remains a production qualification issue, not an invisible guarantee. The current system Rust package also lacks the ASan runtime required for sanitizer fuzzing; non-sanitized coverage fuzz and regression replay have run, but ASan qualification is outstanding.
