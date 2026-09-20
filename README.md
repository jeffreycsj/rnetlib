# RNet Game Networking

RNet is a bounded, event-driven networking foundation for client/server games. The new Rust `rnet-game` facade selects TCP, UDP, or KCP once at listener/connect time, then sends opaque business bytes with `send(session, payload)`; protobuf or another application schema owns its own message type. The server owns the encryption policy and can change it for a live session without changing the client API.

The game facade is under active development, **not yet production-certified**. It currently provides authenticated joins with exact protocol ID/version gating, a no-`msg_type` send/receive API, numeric-IP and hostname connections, server-led plaintext/encrypted transitions, authenticated heartbeats with per-session RTT/jitter samples and aggregate heartbeat percentiles/counters, wire-v2 UDP sequence-gap estimates, KCP retransmission estimates, configurable quality grades and suppressed quality-change events, lifecycle controls, and transport metrics on TCP, UDP, and KCP. Clock synchronization, reconnect tickets, real-time replacement queues, and game-level C/C++11/Go bindings are still pending. The existing transport C ABI, C++11 wrapper, and Go wrapper remain available as advanced lower-level APIs.

See [Game networking quick start](docs/game-networking.md) for the current Rust interface and its security boundaries.

## Implemented capabilities

- TCP, raw UDP, and KCP over UDP on one Tokio runtime, with bounded event/write queues and generation-checked handles.
- Application sessions independent of transport connection state. A TCP accept/connect does not produce `SESSION_OPENED` until the application handshake and server authorization complete.
- `Noise_XX_25519_ChaChaPoly_BLAKE2s` authenticated encryption. Clients pin the server static public key; servers receive the client public key plus encrypted join payload in `AUTH_REQUEST` and call `rnet_session_auth_decide`.
- Stateful encrypted records for ordered TCP and explicit-nonce stateless records with a 64-packet replay window for UDP/KCP.
- HMAC cookie challenges, bound to the source address and a 60-second time bucket, before UDP/KCP Noise/session allocation.
- KCP 0.6.0 with one timer loop per endpoint rather than one thread per session, configured MTU, deadline-aware updates, and reused hot-path buffers. Its core adapter is tested against packet loss and reordering.
- A 28-byte logical frame, incremental TCP framing, Protobuf helpers, metrics, bounded asynchronous logging, static/dynamic libraries, and final-link examples.
- Fixed-memory latency histograms for connect, Noise handshake, authorization wait, send/event queues, KCP RTT/update delay, and logger callbacks. Window snapshots and periodic summaries include P50/P90/P95/P99/P99.9/max in microseconds.
- Configurable per-endpoint admission, handshake deadlines, datagram idle reclamation, and session-isolated event backpressure for high-connection deployments.
- C++11-only public wrapper: no `string_view`, `exchange`, or later language dependency. Binary payloads support `const void * + size_t` and `std::string`.

New deployments should use the transport-neutral `listen`/`connect` APIs. TCP, UDP, or KCP is a configuration value fixed when the endpoint is created; it never appears in the send API. The server chooses the initial application-data security mode and may switch or rekey a live session. Clients follow those authenticated changes automatically.

The old `rnet_endpoint_open`, `rnet_listener_open`, and `rnet_client_join` APIs remain unchanged for ABI compatibility.

## Game API quick start

Rust games use the same listener and send calls for TCP, UDP, and KCP. The transport is selected when the endpoint is created:

```rust
use rnet_core::Transport;
use rnet_game::{GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig};
use rnet_security::Keypair;

let runtime = GameRuntime::new(GameRuntimeConfig::production())?;
let listener = runtime.listen(GameServerConfig {
    transport: Transport::Kcp, // change only this value for TCP or UDP
    bind_addr: "127.0.0.1:7000".parse()?,
    local_key: Keypair::generate()?,
    initial_encryption: true,
    protocol: GameProtocol::new(0x4741_4d45, 1),
})?;
```

Create the client runtime with `GameRuntime::new_with_client_security` and a pinned server public key, then call `connect` with a numeric address or `connect_host` with a hostname. The server receives `GameEvent::AuthRequest` and calls `auth_decide` after validating the join ticket. Both peers receive `GameEvent::SessionReady` before they use `send(session, payload)` and `poll(...)`. No business message type, stream ID, or transport argument is required on send. The server alone can call `set_encryption` or `rekey`; clients follow the authenticated change using the same session handle.

See the [game networking guide](docs/game-networking.md) and the [TCP/UDP/KCP integration test](crates/rnet-game/tests/game_runtime.rs) for client setup, event handling, and security transitions. Raw UDP does not guarantee delivery; choose KCP or TCP if your game requires reliable messages.

## Advanced transport APIs

The underlying `NetworkRuntime` exposes transport-level configuration and events for integrations that need them. Its C ABI, C++11 wrapper, and Go wrapper are available today; game-level bindings for those languages are still planned.

The equivalent public names are:

- C: `rnet_server_open_v2` / `rnet_client_connect_v2`
- C++11: `Runtime::listen` / `Runtime::connect`
- Go: `Runtime.Listen` / `Runtime.Connect`

See [Getting started](docs/getting-started.md) for transport-level Rust, C, C++11, and Go examples, authentication, events, mode changes, latency percentiles, and shutdown order.

## Build and test

Requirements are Rust 1.85 or newer, a C++11 compiler, Go 1.22 or newer, and a Linux-like linker.

```sh
cargo build -p rnet-ffi --release
make check
```

`make check` runs formatting, Clippy, Rust tests, ABI export checks, and the C++11 and Go smoke tests. Formatting and Clippy require the matching rustfmt/clippy components for the active Rust toolchain.

## C lifecycle

1. Initialize `rnet_config_v5_t` with `rnet_config_v5_init`, then create `rnet_runtime_t` with `rnet_runtime_create_v5`. V3/V4 remain frozen for existing callers; V4 added socket tuning and V5 adds runtime-wide endpoint and pending-handshake limits.
2. Generate or load 32-byte X25519 static keys. Use `rnet_keypair_generate` for generation and `rnet_keypair_from_private` to restore the public key from durable private-key storage.
3. A server calls `rnet_server_open_v2`; `rnet_server_config_v2_t.transport` fixes the transport and `initial_security` fixes the initial application-data mode.
4. A client runtime is created with `rnet_runtime_create_v5` and runtime-scoped identity/trust, then calls `rnet_client_connect_v2`. The connect config contains transport, address, and join payload but no encryption selection.
5. The server receives `RNET_EVENT_AUTH_REQUEST`. Event data is `client_public_key[32] || join_payload`; after validation it calls `rnet_session_auth_decide`.
6. Only after acceptance do both sides receive `RNET_EVENT_SESSION_OPENED`. Sending earlier returns `RNET_E_HANDSHAKE_REQUIRED`.
7. Send with `rnet_session_send`; only use `rnet_session_send_ex` when a correlation ID is needed. The legacy `rnet_send` remains ABI-compatible but exposes transport-internal legacy fields.
8. Poll with `rnet_poll_events_ex`, which separates status from event count, and release every nonzero `buffer_token` exactly once. Stop and destroy the runtime when all buffers have been returned.

`rnet_latency_snapshot_v2` returns all latency kinds including P99.9 and can atomically rotate the current window. `rnet_metrics_snapshot_v3` includes endpoint/session/pending-handshake gauges, admission failures by reason, and close counts by stable error code. `rnet_logger_v2_t` emits timestamped structured events. Logging remains bounded, so a slow callback increases `logs_dropped` instead of blocking network work.

The C++11 wrapper exposes `listen`, `connect`, `send`, `poll`, `set_security`, `rekey`, and latency metrics. The Go wrapper exposes the equivalent `Listen`, `Connect`, `Send`, `Poll`, `SetSecurity`, `Rekey`, and typed authentication accessors. Legacy metadata is available only through `send_legacy`/`SendLegacy`.

All pointer/length inputs are borrowed only for the call. Key material is copied into zeroizing Rust storage when it must outlive the call. C/C++/Go applications must also clear their own private-key copies.

## Wire and transport behavior

The logical business frame remains:

```text
magic:u32 | version:u16 | flags:u16 | msg_type:u32 | stream_id:u32
body_len:u32 | request_id:u64 | body:[u8; body_len]
```

For secure sessions the complete logical frame is encrypted, so message type, correlation ID, and body are not visible on the wire. The normal API writes zero to the compatibility `stream_id` and `request_id` fields; advanced correlation reuses `request_id`. TCP adds a bounded record length. UDP carries one encrypted record per datagram. KCP fragments/retransmits the encrypted record and preserves message boundaries.

Raw UDP is intentionally unreliable and unordered. KCP is reliable and ordered but does not supply security itself; RNet's Noise layer is above KCP. Event and send queues, handshake state, KCP buffers, frame sizes, and replay windows are bounded.

## Repository layout

- `crates/rnet-core`: errors, lifecycle, handles, event queue, and buffer ownership.
- `crates/rnet-protocol`: logical frame and Protobuf helpers.
- `crates/rnet-security`: Noise handshake, stateful/stateless encryption, and replay window.
- `crates/rnet-transport`: runtime/state orchestration plus separate TCP, UDP,
  KCP, secure transport, cookie, and metrics modules.
- `crates/rnet-game`: game-facing Rust facade, authenticated join metadata, and opaque payload envelope.
- `crates/rnet-observe`: bounded logger.
- `crates/rnet-ffi`: stable C ABI split by runtime, endpoint, session, events,
  observation, and handle registry responsibilities.
- `include`: the stable C ABI header.
- `cpp`: a small `rnet.hpp` umbrella over typed C++11 headers in `cpp/rnet`.
- `go/rnet`: Go SDK split by config, runtime, endpoint, session, event,
  observation, and keypair responsibilities; `native.h` owns the cgo shims.
- `fuzz`: frame, security-control, cookie/preflight, KCP, game-wire, and FFI configuration fuzz targets.

See [Production deployment](docs/production-deployment.md), [SECURITY.md](SECURITY.md), [dependency-review.md](docs/dependency-review.md), and [adversarial-review.md](docs/adversarial-review.md) before production exposure.

## License

RNet is licensed under the [MIT License](LICENSE).
