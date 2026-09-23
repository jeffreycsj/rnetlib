# Game networking quick start

`rnet-game` is the current game-facing API. It is a production candidate pending target-environment qualification: authenticated heartbeat, quality and clock samples, bounded snapshot replacement, priority/expiry scheduling, correlation/tick metadata, version-range negotiation, and single-runtime one-use reconnect tickets are implemented in Rust, C, C++11 and Go. Cross-instance recovery is intentionally out of scope; 24–72 hour soak and independent protocol/dependency review remain external release gates.

```rust
use rnet_game::{GameProfile, GameProtocol, GameRuntime, GameRuntimeConfig, GameServerConfig};
use rnet_security::Keypair;

let runtime = GameRuntime::new(GameRuntimeConfig::production())?;
let listener = runtime.listen(GameServerConfig::for_profile(
    GameProfile::ReliableRealtime,
    "0.0.0.0:7000".parse()?,
    Keypair::generate()?,
    GameProtocol::new(0x4741_4d45, 1),
))?;
```

Profiles are additive configuration helpers: `Realtime` selects UDP, `ReliableRealtime` selects KCP, and `Session` selects TCP; server helpers enable initial business encryption. Existing explicit transport fields remain available, and neither profile nor transport appears in `send`. C uses `rnet_game_profile_defaults`, C++11 uses `game_server_options` / `game_client_options` (and range variants), and Go uses `Game*ConfigForProfile`.

For clients, construct `ClientSecurity::pinned(client_key, server_public_key)` and pass it to `GameRuntime::new_with_client_security`. Then call `connect(GameClientConfig { ... })` for a numeric address, or `connect_host(GameHostClientConfig { ... })` for a hostname. Both configs select transport once and carry an opaque `join_ticket`; neither exposes an encryption setting.

Poll `runtime.poll(capacity, timeout)` for `GameEvent::AuthRequest`. The library first rejects mismatched protocol ID/version, then exposes the original `join_ticket` and the client's Noise public key to the application. After validating the ticket, call `runtime.auth_decide(session, true)`. Both ends receive `GameEvent::SessionReady` only after authorization.

```rust
runtime.send(session, protobuf_bytes)?;
// GameEvent::Message(message).payload contains the original protobuf bytes.
```

The application defines its packet type in protobuf, FlatBuffers, or another payload schema. The game API has no `msg_type`, `stream_id`, or per-send TCP/UDP/KCP parameter. Optional network-owned sequence and simulation tick metadata use `send_with_options`; they do not determine the business packet type. Raw UDP remains unreliable; use KCP or TCP when the application needs reliable delivery.

```rust
runtime.send_with_options(session, protobuf_bytes, GameSendOptions {
    tick: Some(server_tick),
    correlation_id: request_id,
    priority: GamePriority::High,
    expires_after: Some(std::time::Duration::from_millis(100)),
    ..GameSendOptions::default()
})?;
```

Normal `send` uses normal priority with no expiry or correlation ID. Scheduling is weighted across priorities and round-robin across sessions inside each priority, so a busy player cannot monopolize a class. Expiry applies only while the message remains in the game-stage queue. `scheduled_queue_snapshot()` exposes queue bytes/messages, per-priority admitted/forwarded counts, backpressure requeues and local admission-to-transport P90/P95/P99/max. The tick-specific histogram is the same local queue delay restricted to tick-annotated messages; it is not one-way network latency or tick age.

## Sending and receiving

RNet is poll-driven. `listen` and `connect` create endpoints, but they do not invoke application callbacks. The application must keep calling `poll` for the lifetime of each runtime. The same event stream carries authorization, readiness, received messages, writable notifications, security changes and disconnects; authenticated heartbeat, clock-sync and negotiation controls are consumed internally during the same calls.

The normal flow is:

1. The server polls `AuthRequest`, validates the opaque login ticket, then calls `auth_decide`.
2. Each side stores the session handle from its own `SessionReady` event. Endpoint handles are not valid send targets.
3. Either side calls `send(session, payload)`. The payload already contains the application's protobuf or other business envelope; no message type or transport is passed separately.
4. The peer polls `Message` and decodes `message.payload`. Replies use the receiving `message.session`.
5. `WouldBlock` from `send` means the bounded game scheduler is full. Retain the business message, keep polling so queued work drains, then retry. Backpressure encountered after successful admission is retried internally. Success means staged locally, not acknowledged by the peer.
6. Stop using a handle after `SessionClosed` or after it becomes the old handle in `SessionResumed`.

### Rust

```rust
loop {
    for event in runtime.poll(64, std::time::Duration::from_millis(10)) {
        match event {
            GameEvent::AuthRequest { session, join_ticket, .. } => {
                runtime.auth_decide(session, login_service.accept(join_ticket.as_bytes()))?;
            }
            GameEvent::SessionReady { session, .. } => {
                sessions.insert(session);
            }
            GameEvent::Message(message) => {
                let request = MyProto::decode(message.payload.as_ref())?;
                runtime.send(message.session, make_reply(request).encode_to_vec().as_slice())?;
            }
            GameEvent::Writable { session, .. } => retry_pending(session),
            GameEvent::SessionClosed { session, reason, .. } => disconnect(session, reason),
            _ => {}
        }
    }
}
```

[`game_quickstart.rs`](../crates/rnet-game/examples/game_quickstart.rs) is a complete executable client/server echo. Run it with:

```sh
cargo run -p rnet-game --example game_quickstart
```

### C++11

`GameRuntime::poll` returns owned event vectors, so payload bytes remain valid after the native poll call:

```cpp
for (const rnet::GameEvent &event : runtime.poll(64, 10)) {
  if (event.type == RNET_GAME_AUTH_REQUEST) {
    runtime.auth_decide(event.session, validate_ticket(event.data));
  } else if (event.type == RNET_GAME_SESSION_READY) {
    runtime.send(event.session, encode_login_complete());
  } else if (event.type == RNET_GAME_MESSAGE) {
    const Request request = decode_request(event.data);
    runtime.send(event.session, encode_reply(request));
  } else if (event.type == RNET_GAME_SESSION_CLOSED) {
    on_disconnect(event.session, event.status);
  }
}
```

See [`game_echo_smoke.cpp`](../examples/cpp/game_echo_smoke.cpp) for a compiled C++11 example.

### Go

`Poll` copies event payloads into Go-owned byte slices before releasing native buffers:

```go
events, err := runtime.Poll(64, 10*time.Millisecond)
if err != nil { return err }
for _, event := range events {
    switch event.Type {
    case GameAuthRequest:
        if err := runtime.AuthDecide(event.Session, validateTicket(event.Data)); err != nil { return err }
    case GameSessionReady:
        sessions[event.Endpoint] = event.Session
    case GameMessage:
        request := decodeRequest(event.Data)
        if err := runtime.Send(event.Session, encodeReply(request)); err != nil { return err }
    case GameSessionClosed:
        onDisconnect(event.Session, event.Status)
    }
}
```

See [`game_test.go`](../go/rnet/game_test.go) for TCP, UDP and KCP examples.

### C ABI

The C ABI borrows send bytes only for the duration of `rnet_game_send`. Poll payloads remain owned by the runtime until both nonzero event tokens are released. Process or copy the bytes first:

```c
rnet_game_event_t events[64];
size_t count = 0;
int32_t status = rnet_game_poll_events(runtime, events, 64, 10, &count);
if (status != RNET_OK) return status;

for (size_t i = 0; i < count; ++i) {
  rnet_game_event_t *event = &events[i];
  if (event->event_type == RNET_GAME_AUTH_REQUEST) {
    rnet_game_auth_decide(runtime, event->session,
                          validate_ticket(event->data, event->data_len));
  } else if (event->event_type == RNET_GAME_MESSAGE) {
    handle_message(event->session, event->data, event->data_len);
    rnet_slice_t reply = build_reply();
    rnet_game_send(runtime, event->session, reply);
  }
  if (event->buffer_token != 0)
    rnet_game_buffer_release(runtime, event->buffer_token);
  if (event->aux_buffer_token != 0)
    rnet_game_buffer_release(runtime, event->aux_buffer_token);
}
```

For C, C++11 and Go, keep exactly one logical poll dispatcher per runtime and route copied events to gameplay workers. Sends may originate from application workers, but business code must define how it queues and retries `WouldBlock` without blocking the poll dispatcher.

`send_latest(session, key, payload)` and `send_latest_with_tick(...)` stage replaceable snapshots in a separate bounded queue. Keys are scoped to one session; replacing a pending key releases its previous budget and preserves fair key order. The queue flushes on `poll`, or the game tick can call `flush_realtime(capacity)` explicitly. If the transport queue is full, the pending key rotates behind others instead of blocking the event loop. `RealtimeQueueConfig` caps total bytes, per-session bytes, keys per session, and each flush batch. `realtime_queue_snapshot()` and Prometheus expose queued bytes/messages, game-stage replacements, close/backpressure drops, send failures, and forwarded totals. For TCP/KCP, a keyed slot can also be replaced after transport admission while it is still waiting for the I/O worker; `transport_latest_snapshot()` reports those replacements, worker pickups and admission failures split by stable reason. Worker pickup is the recall boundary: a partly written TCP record or data already given to KCP cannot be withdrawn, including KCP's own deferred/retransmission queues. A pickup does **not** prove delivery; a later I/O failure is traced through the existing session-close/error metrics. UDP retains game-stage-only replacement. Use ordinary `send` for messages that must not be dropped or replaced. A successful forward means queued locally, **not delivered**.

## Single-runtime reconnect

After ordinary authorization and `SessionReady`, the server may call `issue_resume_ticket(server_session, identity)` with an opaque 1–64-byte application identity. The client receives `GameEvent::ResumeTicket`; keep its bytes private and call `connect_resume(config, old_client_session, ticket.as_bytes())` or `connect_host_resume(...)` after interruption. These methods perform a **new Noise handshake** and use an explicit `RGR3` resume join; ordinary joins use `RGV3`. The same client static key, listener, protocol ID, and version are required. Old wire-v2 markers are rejected.

The server receives `GameEvent::ResumeRequest` containing the claimed old server handle and identity, the new session, and the new join proof. It must recheck application identity and call `auth_decide(new_session, true)` or `false`. Only success emits `SessionResumed { old_session, new_session }` on each side; this event means the new handle is ready, and the old handle is invalid. For wire v4, authorization alone does not invalidate a still-live old route: takeover waits for the protected SELECT/ACK/READY exchange, and negotiation failure leaves the old route usable. A denied request never changes the old authorized session. Explicit `close_session`/`close_endpoint` revokes applicable tickets and pending claims. Tickets are valid once for 30 seconds by default; `with_resume(ttl, max_tickets)` changes the bounded policy (maximum TTL: five minutes). `resume_metrics_snapshot()` and `rnet_game_resume_*` Prometheus metrics expose issued, received, rejected, denied, revoked, resumed, and outstanding counts without player labels.

Tickets and replay state live only in one `GameRuntime`. Restarting it or connecting to another server instance requires a full login; copying the signing key alone would not preserve single-use protection. Built-in `Debug` output redacts keys, tickets, authentication bytes, and event payloads, but applications must still avoid logging credential bytes they explicitly extract. See [game_resume.rs](../crates/rnet-game/tests/game_resume.rs) for the TCP/UDP/KCP flow, replay denial, application rejection, kick revocation, and hostname reconnect.

For operations, `metrics_snapshot`, `prometheus_snapshot`, `latency_snapshot`, and `drain_latency_window` expose the transport's bounded resource gauges and latency percentiles. `network_quality(session)` returns the latest authenticated heartbeat RTT, smoothed RTT, jitter, and sample count (`None` before the first reply). `heartbeat_metrics_snapshot()` returns cumulative probe/reply/timeout counters and aggregate RTT P50/P90/P95/P99/P99.9; `drain_heartbeat_rtt_window()` rotates a separate RTT window for short-lived incidents without clearing cumulative data. `prometheus_snapshot()` includes the cumulative game heartbeat metrics under `rnet_game_heartbeat_*` names without per-player labels. Only timely, challenge-matched replies contribute RTT samples; a zero sample count means percentiles are not yet meaningful. `close_session`, `close_endpoint`, and `stop` are available on `GameRuntime`; game code does not need to hold a second, lower-level runtime object.

Rust callers may optionally attach a `BoundedLogger` with `GameRuntime::new(config)?.with_logger(logger)` (also available after `new_with_client_security`). It emits structured `rnet.game` records for authorization, resume, protocol rejection, ready/close, join failure, security, and quality events, with a process-local runtime ID, endpoint/session/transport, and error code. Transport is `0` if the endpoint was already removed before an event was polled. The logger receives no join/resume ticket, player identity, or business payload bytes; `Message`, `Writable`, and every snapshot replacement are intentionally not logged. `game_logger_snapshot()` reports queue drops and sink panics (`None` when disabled); Prometheus exposes `rnet_game_logger_enabled`, `rnet_game_logger_dropped_total`, and `rnet_game_logger_sink_panics_total`. Logging is driven by `poll`; the bounded callback runs on its own thread. Real-time queue drop/replacement counts remain in metrics, not per-packet logs.

```rust
use rnet_game::{BoundedLogger, GameRuntime, GameRuntimeConfig, LoggerConfig};

let logger = BoundedLogger::new(LoggerConfig::default(), |record| {
    eprintln!("{} endpoint={} session={} code={:?}",
        record.event_name, record.endpoint, record.session, record.error_code);
})?;
let runtime = GameRuntime::new(GameRuntimeConfig::production())?.with_logger(logger);
```

The example writes to stderr for clarity; use a nonblocking structured sink in a deployment and alert on logger drops or sink panics.

With a logger configured, polling also emits `game_latency_summary` every 30 seconds by default. It includes cumulative count/P90/P95/P99/P99.9/max (microseconds) for heartbeat RTT, transport phases, scheduling and logger callbacks, plus queue gauges and logger failures. Zero samples are not evidence of zero latency. Change the period with `set_metrics_log_interval(Duration)` in Rust, `rnet_game_metrics_log_interval_set(runtime, milliseconds)` in C, `set_metrics_log_interval(milliseconds)` in C++11, or `SetMetricsLogInterval(time.Duration)` in Go; zero disables the summary. Lifecycle errors preserve available library-generated causes, but credentials and application bytes never become diagnostics. Already-established close events may contain only their stable reason code.

The [C# SDK guide](csharp.md) covers the same game lifecycle through .NET 8 on Linux x64, including complete send/receive loops, injected loggers, dynamic encryption and managed/native ownership. Its `SetMetricsLogInterval(milliseconds)` and `PrometheusSnapshot()` require the matching native library from the same package.

Game wire v3 joins use `RGV3` and intentionally reject older peers; opt-in wire v4 uses a distinct marker. During v4's final confirmation, early business data is held behind the ready event with per-session limits and a runtime-wide count/byte budget derived from `event_queue_capacity` and `max_event_bytes`; its reservation remains charged until application polling or session cleanup. `range_buffer_snapshot()` reports current/peak/limit gauges and per-session/runtime rejection totals, and the same low-cardinality series use the `rnet_game_range_early_*` Prometheus prefix. Raw UDP business messages carry a network-owned sequence extension separate from the optional application sequence. `udp_loss_snapshot(session)` reports receiver-side gaps within a 64-packet reorder window; a failed local send does not consume a sequence. `kcp_retransmission_snapshot(session)` reports per-session PUSH retransmissions independently of heartbeat availability. `network_quality(session)` includes `grade`, `basis`, optional `udp_loss`, and optional `kcp_retransmissions`; `QualityPolicy` can tune grade thresholds. KCP counts PUSH segments sent again before acknowledgement and uses a rolling 128-segment window. `GameEvent::QualityChanged` requires two consecutive non-unknown samples at a changed grade/basis and suppresses repeats. `basis=LatencyOnly` means no transport loss signal was used (including TCP). Neither UDP gaps nor KCP retransmissions are an IP-layer packet capture, and low sample counts do not establish a reliable loss rate. Heartbeat samples remain authenticated even if business records are plaintext, but UDP business-data sequences in that mode are **not integrity-protected** and must be treated as advisory rather than a trusted security or billing signal. Clients automatically exchange protected four-timestamp `ClockSync` controls after game readiness. `clock_sync_snapshot(session)` yields an optional server-minus-client offset and network RTT; `clock_micros()` gives the runtime-local monotonic origin. The sample uses process-local monotonic clocks and includes local/network queuing, so it is only an approximate transport sample, never UTC or a trusted anti-cheat time source. Rust returns `None` without a sample; C uses `rnet_game_clock_sync_t.available` as the has-sample bit (`0` on server sessions), with equivalent `available`/`Available` fields in C++11/Go. Tick-annotated local scheduler delay is reported separately from clock/network RTT and must not be interpreted as anti-cheat time.

```rust
// On a ready client session, after continuing to poll both endpoints:
if let Some(sample) = client.clock_sync_snapshot(client_session)? {
    let estimated_server_micros = i128::from(client.clock_micros())
        + i128::from(sample.server_minus_client_us);
    println!("estimated server monotonic microseconds: {estimated_server_micros}");
}
```

Heartbeats run automatically for ready sessions while the application continuously calls `poll`. The production default is a 5-second interval and 15-second reply timeout; `GameRuntimeConfig::production().with_heartbeat(interval, timeout)` tunes both. A missing acknowledgement closes the session with `Timeout`. Every `poll` batch advances liveness scheduling even if it contains only hidden authenticated controls; an on-time queued acknowledgement is processed first. A completed wire-v4 business backlog cannot block this control processing. `poll(0, ...)` is a nonblocking maintenance tick: it returns no events but still flushes bounded work and advances heartbeat, clock, and negotiation state; C has the same behavior with a null event pointer and capacity zero. Heartbeat controls always use Noise protection, including when business payloads are plaintext, and never appear as `GameEvent::Message`. The reported RTT is measured from local enqueue to authenticated event receipt, so a congested local send queue contributes to the value; do not interpret it as pure wire latency. If the application stops polling entirely, heartbeat replies cannot be processed and liveness checks do not advance.

## Server-controlled encryption

With `GameRuntimeConfig::production()`, business plaintext is disabled. A deployment that explicitly accepts the risk can use `GameRuntimeConfig::production().allow_plaintext_business_data(true)` while retaining the production policy that blocks unauthenticated legacy endpoints. The server can then call `runtime.set_encryption(session, false)` and later `runtime.set_encryption(session, true)`; clients follow authenticated security-control messages automatically. Plaintext business records genuinely lack confidentiality and integrity: an on-path party can alter valid application payloads. A malformed envelope or forged control kind from the unverified plaintext business channel is silently dropped and increments `rnet_game_plaintext_invalid_envelopes_dropped_total`; it does not produce a per-packet log/event that an attacker could amplify into the game loop. The same error in a Noise-protected business record still closes the session and publishes `ProtocolViolation`, even after a live mode switch. Authenticated-control errors also close the session. Corrupting outer record framing can still cause a transport-level disconnect, so plaintext provides no availability or integrity guarantee. Avoid plaintext on public networks. Authentication and mode-switch control stay protected by Noise.

The server may call `runtime.rekey(session)` to rotate established session keys without changing whether business data is encrypted. Clients cannot initiate a rekey. `SecurityChanged` reports the completed operation and security epoch; the application continues using the same session handle and `send` API.

The original `listen` / `connect` / `connect_resume` gate remains exact-version wire v3: client and server must configure the same nonzero protocol ID and version. Range mode is explicitly opt-in: `GameProtocolRange::new(id, min, max)` with `listen_range`, `connect_range` / `connect_host_range`, and corresponding `connect_range_resume` methods uses wire v4. The server chooses the highest version in the intersection before business authorization and authenticates its choice with a session-bound nonce, protocol ID, and Noise-protected SELECT/ACK/READY exchange. `selected_protocol_version(session)` is available to the server during `AuthRequest` and to the client after selection. No overlap is rejected before business authorization; v3 and v4 never silently mix. A resumed range session keeps the ticket's original selected version, gets a new handle and requires new business authorization; the new client range must still include that version. `build_id` and `capabilities` remain authenticated metadata for application authorization, not permission to activate network-library features. See [game_range.rs](../crates/rnet-game/tests/game_range.rs) for TCP/UDP/KCP and resume cases.

The complete live-session test in [game_runtime.rs](../crates/rnet-game/tests/game_runtime.rs) covers TCP, UDP, KCP, plaintext-to-encrypted-to-plaintext transitions, rekey, hostname joining, and version rejection.

## C, C++11, and Go basic game APIs

The additive `rnet_game_*` C functions use separate game runtime handles; do not pass them to the older `rnet_runtime_*` transport functions. `rnet_abi_version()` remains `1`: the game functions add symbols and structs without changing existing layouts. Use `rnet_game_config_init`, then `rnet_game_runtime_create` with optional `rnet_client_security_t` for a pinned client identity. A server uses `rnet_game_server_listen` with transport, protocol ID/version, key, and initial encryption. A client uses `rnet_game_client_connect` with transport, hostname/IP, protocol ID/version, and opaque join ticket; it does not choose encryption. For opt-in wire v4, use `rnet_game_range_server_config_t` / `rnet_game_range_client_config_t` with `rnet_game_server_listen_range`, `rnet_game_client_connect_range`, or `rnet_game_client_resume_connect_range`; `rnet_game_selected_protocol_version` returns the server-selected version. After `RNET_GAME_AUTH_REQUEST`, the server calls `rnet_game_auth_decide`; only `RNET_GAME_SESSION_READY` handles may send using `rnet_game_send(runtime, session, payload)`. Poll with `rnet_game_poll_events`, and release both nonzero event buffer tokens with `rnet_game_buffer_release` before destroying the runtime. Credential buffers are erased by the C layer when released; application-side copies remain the application's responsibility. For same-runtime reconnection, a ready server session calls `rnet_game_issue_resume_ticket`, the client receives `RNET_GAME_RESUME_TICKET`, then calls the matching exact- or range-version resume connect with its old local handle. The server must authorize `RNET_GAME_RESUME_REQUEST` again; only then does each side receive `RNET_GAME_SESSION_RESUMED` with its own old handle in `related_session`.

C++11 users can include `rnet.hpp` and construct `rnet::GameRuntime` with a client key and pinned server key, then call `listen`, `connect`, `auth_decide`, `send`, and `poll`. [`game_echo_smoke.cpp`](../examples/cpp/game_echo_smoke.cpp) is a runnable TCP example; the same methods work for UDP and KCP by changing only `GameServerOptions.transport` and `GameClientOptions.transport`. Explicit range joins use `GameRangeServerOptions` / `GameRangeClientOptions`, `listen_range`, `connect_range` or `connect_range_resume`, and `selected_protocol_version`; see [`game_range_smoke.cpp`](../examples/cpp/game_range_smoke.cpp). `GameRuntime::poll` copies event bytes into owned vectors and releases native tokens even if a C++ allocation throws.

Go users can call `NewGameRuntime(&ClientSecurity{LocalKey: clientKey, ExpectedServerPublicKey: serverKey.Public[:]})`, then `Listen(GameServerConfig{...})`, `Connect(GameClientConfig{...})`, `AuthDecide`, `Send`, and `Poll`. [`game_test.go`](../go/rnet/game_test.go) shows authorization, opaque-payload exchange, and one-use recovery for TCP, UDP, and KCP. Explicit range joins use `GameProtocolRange`, `ListenRange`, `ConnectRange` / `ConnectRangeResume`, and `SelectedProtocolVersion`. Go `Poll` copies native event data and returns all C tokens before returning. `GameEvent.Data` may contain a join credential or resume ticket; default `%v`/`%#v` formatting of game events and configs redacts those bytes, but do not log the byte slices explicitly and erase your own copies when appropriate. Go's zero-value `GameServerConfig.InitialSecurity` means encrypted; choose `SecurityPlaintext` explicitly and enable `GameConfig.AllowPlaintextBusinessData` only when required.

This cross-language slice covers the ordinary and range-version game flows, server encryption switching, session/endpoint close, rekey, `LatestOnly` staging, single-runtime resume, event metadata, per-session quality snapshots, runtime-wide real-time queue counters, cumulative heartbeat/resume/clock/logger metrics, optional bounded structured game-event logging, and a no-player-label Prometheus snapshot. `rnet_game_network_quality` reports `available=0` before the first authenticated RTT sample; its separate availability flags distinguish TCP's unavailable loss from UDP sequence gaps and KCP retransmission ratios. `rnet_game_realtime_queue_snapshot` (C), `realtime_queue_snapshot()` (C++11), and `RealtimeQueueSnapshot()` (Go) report current staged-message/byte gauges plus cumulative admission-rejected, replaced, closed-dropped, backpressure-dropped, send-failed, and forwarded counts. `rnet_game_range_buffer_snapshot` (C), `range_buffer_snapshot()` (C++11), and `RangeBufferSnapshot()` (Go) report the wire-v4 early-data current/peak/limit gauges and per-session/runtime rejection totals. `rnet_game_transport_latest_snapshot` (C), `transport_latest_snapshot()` (C++11), and `TransportLatestSnapshot()` (Go) report TCP/KCP pending replacements, irreversible worker pickups, and admission failures split into would-block, invalid-handle/state, handshake-required, unsupported, too-large and other. The older replacement-only queries remain available. Prometheus exports the same cumulative counters. Forwarded and pickup mean local handoff, **not delivery**. `rnet_game_metrics_snapshot` (C), `metrics_snapshot()` (C++11), and `MetricsSnapshot()` (Go) expose cumulative game counters, RTT percentiles and a `logger_available` flag; callback latency is measured on the asynchronous logger thread. These runtime-wide metrics have no per-player labels and require no buffer release. Go callback panics cannot cross the C boundary; the Go facade recovers them and adds them to `MetricsSnapshot().Logger.SinkPanics` and a separate `rnet_game_go_logger_panics_total` Prometheus counter. Release the Prometheus C buffer token after reading. For capacity tuning, set `rnet_game_config_t.network_config` to a borrowed `rnet_config_v5_t` while creating the C runtime; C++11 has a `GameRuntime(config, ...)` constructor, and Go accepts `GameConfig{Network: &network}` where `network` starts from `DefaultConfig()`. The game-specific plaintext flag remains authoritative; legacy unauthenticated endpoints and transport-level logger callbacks are rejected. Configure a game logger at creation with `rnet_game_runtime_create_logged(..., rnet_logger_v2_t*, ...)`, the C++11 constructor accepting `rnet_logger_v2_t`, or Go `GameConfig{Logger: callback}`. The callback may query metrics but must not stop/destroy its runtime; its user data must stay valid until destroy returns. It must return promptly, because destroy waits for the bounded dispatcher to finish. Do not substitute the old transport API on a game runtime handle.

For advanced sends, C uses `rnet_game_send_ex` plus `rnet_game_poll_events_v2`; C++11 and Go use `GameSendOptions` and expose the returned correlation ID. `rnet_game_scheduled_queue_snapshot` and its C++11/Go wrappers expose per-priority and queue-delay telemetry. Queue budgets are configured additively with `rnet_game_config_v2_t` and `rnet_game_runtime_create_v2`; the V1 struct remains frozen. C++11 accepts either V1 or V2 raw configuration, while Go uses V2 internally and exposes `RealtimeQueue` and `ScheduledQueue` fields on `GameConfig`.

## Current production limits

Adaptive KCP negotiates a fresh random conversation during stateless cookie preflight, so delayed packets from a previous connection at the same address cannot enter a new engine. The pinned KCP dependency is compiled with wrapping arithmetic only for that package, matching KCP's modulo-2^32 serial semantics in debug, test and release builds; RNet itself keeps overflow checks. Boundary and stale-conversation regressions are included. Legacy low-level `EndpointConfig::secure_kcp_*` remains an ABI-compatibility path with its historical fixed conversation; new game and transport-neutral deployments use adaptive `listen`/`connect`. All six fuzz boundaries have completed short AddressSanitizer campaigns on the pinned nightly, and the pure core/protocol suites pass Miri. These smoke gates do not replace long fuzzing, target-environment weak-network soak, or independent Noise/KCP review.
