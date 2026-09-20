# Game networking quick start (Rust)

`rnet-game` is the current game-facing API. It is a production candidate, not a completed game SDK: authenticated heartbeat, per-session RTT/jitter, UDP sequence-gap and KCP retransmission estimates, quality grades, bounded pre-transport real-time snapshot replacement, and single-runtime one-use reconnect tickets are available. Clock synchronization, cross-instance recovery, replacement inside already-admitted TCP/KCP transport queues, and game-level C/C++11/Go bindings remain unfinished. The transport library underneath still exposes its existing C/C++11/Go APIs.

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

`send_latest(session, key, payload)` and `send_latest_with_tick(...)` stage replaceable snapshots in a separate bounded queue. Keys are scoped to one session; replacing a pending key releases its previous budget and preserves fair key order. The queue flushes on `poll`, or the game tick can call `flush_realtime(capacity)` explicitly. If the transport queue is full, the pending key rotates behind others instead of blocking the event loop. `RealtimeQueueConfig` caps total bytes, per-session bytes, keys per session, and each flush batch. `realtime_queue_snapshot()` and Prometheus expose queued bytes/messages, replacements, close/backpressure drops, send failures, and forwarded totals. This is **best-effort coalescing before transport enqueue** across TCP/UDP/KCP; once forwarded, an older TCP write or KCP retransmission cannot be recalled. Use ordinary `send` for messages that must not be dropped or replaced.

## Single-runtime reconnect

After ordinary authorization and `SessionReady`, the server may call `issue_resume_ticket(server_session, identity)` with an opaque 1–64-byte application identity. The client receives `GameEvent::ResumeTicket`; keep its bytes private and call `connect_resume(config, old_client_session, ticket.as_bytes())` or `connect_host_resume(...)` after interruption. These methods perform a **new Noise handshake** and use an explicit `RGJ3` resume join; ordinary joins remain `RGJ2`. The same client static key, listener, protocol ID, and version are required.

The server receives `GameEvent::ResumeRequest` containing the claimed old server handle and identity, the new session, and the new join proof. It must recheck application identity and call `auth_decide(new_session, true)` or `false`. Only success emits `SessionResumed { old_session, new_session }` on each side; this event means the new handle is ready, and the old handle is invalid. A denied request never changes the old authorized session. Explicit `close_session`/`close_endpoint` revokes applicable tickets and pending claims. Tickets are valid once for 30 seconds by default; `with_resume(ttl, max_tickets)` changes the bounded policy (maximum TTL: five minutes). `resume_metrics_snapshot()` and `rnet_game_resume_*` Prometheus metrics expose issued, received, rejected, denied, revoked, resumed, and outstanding counts without player labels.

Tickets and replay state live only in one `GameRuntime`. Restarting it or connecting to another server instance requires a full login; copying the signing key alone would not preserve single-use protection. Built-in `Debug` output redacts keys, tickets, authentication bytes, and event payloads, but applications must still avoid logging credential bytes they explicitly extract. See [game_resume.rs](../crates/rnet-game/tests/game_resume.rs) for the TCP/UDP/KCP flow, replay denial, application rejection, kick revocation, and hostname reconnect.

For operations, `metrics_snapshot`, `prometheus_snapshot`, `latency_snapshot`, and `drain_latency_window` expose the transport's bounded resource gauges and latency percentiles. `network_quality(session)` returns the latest authenticated heartbeat RTT, smoothed RTT, jitter, and sample count (`None` before the first reply). `heartbeat_metrics_snapshot()` returns cumulative probe/reply/timeout counters and aggregate RTT P50/P90/P95/P99/P99.9; `drain_heartbeat_rtt_window()` rotates a separate RTT window for short-lived incidents without clearing cumulative data. `prometheus_snapshot()` includes the cumulative game heartbeat metrics under `rnet_game_heartbeat_*` names without per-player labels. Only timely, challenge-matched replies contribute RTT samples; a zero sample count means percentiles are not yet meaningful. `close_session`, `close_endpoint`, and `stop` are available on `GameRuntime`; game code does not need to hold a second, lower-level runtime object.

Game wire v2 joins are marked `RGJ2` and intentionally reject v1 peers. Raw UDP business messages carry a network-owned sequence extension separate from the optional application sequence. `udp_loss_snapshot(session)` reports receiver-side gaps within a 64-packet reorder window; a failed local send does not consume a sequence. `kcp_retransmission_snapshot(session)` reports per-session PUSH retransmissions independently of heartbeat availability. `network_quality(session)` includes `grade`, `basis`, optional `udp_loss`, and optional `kcp_retransmissions`; `QualityPolicy` can tune grade thresholds. KCP counts PUSH segments sent again before acknowledgement and uses a rolling 128-segment window. `GameEvent::QualityChanged` requires two consecutive non-unknown samples at a changed grade/basis and suppresses repeats. `basis=LatencyOnly` means no transport loss signal was used (including TCP). Neither UDP gaps nor KCP retransmissions are an IP-layer packet capture, and low sample counts do not establish a reliable loss rate. Heartbeat samples remain authenticated even if business records are plaintext, but UDP business-data sequences in that mode are **not integrity-protected** and must be treated as advisory rather than a trusted security or billing signal. Clock sync and tick-delay metrics are not implemented yet.

Heartbeats run automatically for ready sessions while the application continuously calls `poll`. The production default is a 5-second interval and 15-second reply timeout; `GameRuntimeConfig::production().with_heartbeat(interval, timeout)` tunes both. A missing acknowledgement closes the session with `Timeout`. Heartbeat controls always use Noise protection, including when business payloads are plaintext, and never appear as `GameEvent::Message`. The reported RTT is measured from local enqueue to authenticated event receipt, so a congested local send queue contributes to the value; do not interpret it as pure wire latency. If the application stops polling, heartbeat replies cannot be processed and liveness checks do not advance.

## Server-controlled encryption

With `GameRuntimeConfig::production()`, business plaintext is disabled. A deployment that explicitly accepts the risk can use `GameRuntimeConfig::production().allow_plaintext_business_data(true)` while retaining the production policy that blocks unauthenticated legacy endpoints. The server can then call `runtime.set_encryption(session, false)` and later `runtime.set_encryption(session, true)`; clients follow authenticated security-control messages automatically. Plaintext business records genuinely lack confidentiality and integrity. Authentication and mode-switch control stay protected by Noise.

The server may call `runtime.rekey(session)` to rotate established session keys without changing whether business data is encrypted. Clients cannot initiate a rekey. `SecurityChanged` reports the completed operation and security epoch; the application continues using the same session handle and `send` API.

The exact-version join gate is not a downgrade negotiation: client and server must configure the same nonzero protocol ID and version. `build_id` and `capabilities` are authenticated metadata for application authorization, not permission to activate network-library features. Future version-range negotiation requires a server-authenticated selected-version response and is not implemented yet.

The complete live-session test in [game_runtime.rs](../crates/rnet-game/tests/game_runtime.rs) covers TCP, UDP, KCP, plaintext-to-encrypted-to-plaintext transitions, rekey, hostname joining, and version rejection.

## Current production limits

The KCP adapter rejects malformed values near the pinned dependency's signed sequence boundary, but automatic rotation of an exceptionally long-lived KCP conversation is not implemented. Deployments approaching two billion packets on one KCP session must re-establish that session before the boundary. This remains a production qualification issue, not an invisible guarantee. The current system Rust package also lacks the ASan runtime required for sanitizer fuzzing; non-sanitized coverage fuzz and regression replay have run, but ASan qualification is outstanding.
