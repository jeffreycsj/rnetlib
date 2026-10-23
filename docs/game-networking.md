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
    GameProtocol::new(0x4741_4d45, 1, 1),
))?;
```

Profiles are additive configuration helpers: `Realtime` selects UDP, `ReliableRealtime` selects KCP, and `Session` selects TCP; server helpers enable initial business encryption. Existing explicit transport fields remain available, and neither profile nor transport appears in `send`. C uses `rnet_game_profile_defaults`, C++11 uses `game_server_options` / `game_client_options`, and Go uses `Game*ConfigForProfile`. Business versions use an inclusive min/max range on the same API; equal bounds request an exact version, not an older SDK or a separate wire path.

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

Poll with **positive capacity regularly on both peers**. Zero-capacity calls still drive timers and bounded maintenance, but cannot replace event consumption. Ordinary public events waiting for the application have independent count and payload-byte limits inherited from `event_queue_capacity` and `max_event_bytes`. Polling only takes the transport FIFO prefix that can fit the remaining staging budget; it does not drop or bypass a non-fitting head. Draining public events returns that budget. The early-data negotiation budget and transport event queue remain separately bounded, so these limits are not a whole-process memory cap and do not include returned application-owned events or allocation overhead. `range_buffer_snapshot()` still reports early data only, not ordinary public backlog.

When the next event cannot fit, transport-event draining pauses while heartbeat, clock and negotiation timers continue. ACKs behind the FIFO head cannot bypass that backpressure; prolonged zero-only polling can therefore time out a healthy peer. This does not add a new staging-full kick or eviction policy. Existing transport-level policies still apply: TCP waits for event capacity, while UDP/KCP close the affected session if a per-session event cannot enter the native event queue. Priority lifecycle events may evict data notifications, or be rejected when only lifecycle history remains. Bounded game staging is not a guarantee of lossless delivery or continued connection under overload.

The normal flow is:

1. The server polls `AuthRequest`, validates the opaque login ticket, then calls `auth_decide`.
2. Each side stores the session handle from its own `SessionReady` event. Endpoint handles are not valid send targets.
3. Either side calls `send(session, payload)`. The payload already contains the application's protobuf or other business envelope; no message type or transport is passed separately.
4. The peer polls `Message` and decodes `message.payload`. Replies use the receiving `message.session`.
5. `WouldBlock` from `send` means the bounded game scheduler is full. Retain the business message, keep polling so queued work drains, then retry. Backpressure encountered after successful admission is retried internally. Success means staged locally, not acknowledged by the peer.
6. Stop using a handle after `SessionClosed` or after it becomes the old handle in `SessionResumed`.

### Rust

The dispatch sketch below omits application retry storage and fails fast on send errors.
Use the executable example linked below for bounded `WouldBlock` handling; `Writable`
only signals transport capacity and is not the sole trigger for retrying game-stage admission.

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

[`game_quickstart.rs`](../crates/rnet-game/examples/game_quickstart.rs) is a complete executable client/server echo with logging, bounded application retry storage and explicit two-runtime shutdown. Run it with:

```sh
cargo run -p rnet-game --example game_quickstart
cargo run -p rnet-game --example game_quickstart -- tcp --one-slot
cargo run -p rnet-game --example game_quickstart -- udp
cargo run -p rnet-game --example game_quickstart -- kcp --deny  # Expected AuthRejected, nonzero exit
cargo test -p rnet-game --example game_quickstart
```

No arguments selects KCP. `--one-slot` limits the real game scheduler to one message,
so the binary request is accepted and the empty request initially encounters backpressure.
The application keeps at most two copied messages of at most 128 bytes each per peer,
removes only admitted messages and retries after positive-capacity polling, not a busy loop.
Both echoes must match their original bytes before the example reports success.
This small request/response workload is not a general multi-player queue implementation.

The exchange has a ten-second deadline; each runtime's stop retry has a separate
five-second budget. Deadlines do not preempt a slow runtime call or logging sink.
Both stops are attempted even after a failure, and cleanup cannot turn an exchange error
into success. Join/protocol/close errors preserve their category. The example deliberately
aborts on any endpoint error, including potentially recoverable errors; production can query
endpoint validity and choose its own failover policy. UDP loss, duplication or reordering can
fail this strict echo, and the example does not add retransmission to UDP.

The logger adapter prints library diagnostics, never the echoed payload or ticket. Production
must replace the demo credential check, persistent server-key setup and stderr sink with its
own services. P95/P99 summaries retain the runtime's default 30-second poll-driven interval,
so a short successful echo normally finishes before a summary appears. The same source is
compiled and executed by rustdoc; its boundary and real-network tests also run in the workspace.

### C++11

`GameRuntime::poll` returns owned event vectors, so payload bytes remain valid after the native poll call. The dispatch sketch below omits application retry policy and lets send exceptions escape; use the complete example below for bounded backpressure handling:

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

[`game_quickstart.cpp`](../examples/cpp/game_quickstart.cpp) is the complete two-runtime C++11 echo. Its [support header](../examples/cpp/game_quickstart_support.hpp) contains the small application retry/cleanup policy, not additional SDK methods. Existing [`game_echo_smoke.cpp`](../examples/cpp/game_echo_smoke.cpp) remains the separate resume regression.

```sh
make cpp-test
./target/rnet-cpp-game-quickstart tcp --one-slot
./target/rnet-cpp-game-quickstart udp
./target/rnet-cpp-game-quickstart kcp --deny       # Expected authorization failure, exit 1
./target/rnet-cpp-game-quickstart tcp --wrong-key # Expected handshake/trust failure, exit 1
```

No arguments defaults to KCP. `--one-slot` uses a real one-message scheduler budget,
while each peer keeps at most two application-owned copies of up to 128 bytes each.
On `rnet::Error` with `status() == RNET_E_WOULD_BLOCK`, the example stops that flush,
polls both peers with positive capacity and retries only the unaccepted suffix. Other native
errors and allocation exceptions propagate without silently discarding pending work. Neither
the empty payload nor the binary payload is interpreted as a NUL-terminated string.

The exchange has a ten-second deadline; each checked close retry has a separate five-second
budget. Both runtimes are explicitly closed even if the first close throws, and the original
exchange exception is retained while cleanup errors are reported separately. RAII destructors
are fallback cleanup, not proof of success. Deadlines do not preempt slow calls or logger sinks.
In the wrong-pin scenario the server can report handshake EOF before the client pin error is
polled; the demo preserves that first local cause instead of promising one error code across peers.

The literal credential, fresh server key and stderr logger are demonstration choices, not
production authentication/key management/logging. The callback must not throw across C or close
its runtime. It receives library diagnostics, not application bytes or tickets; the default
30-second P95/P99 summary is not expected during every short echo. UDP loss, duplication or
reordering can fail this strict echo. The demo also aborts on recoverable endpoint errors;
real applications should choose their own bounded overload and failover policy.

Manual Linux compilation (from the repository root):

```sh
cargo build -p rnet-ffi
g++ -std=c++11 -Wall -Wextra -Werror -Iinclude -Icpp examples/cpp/game_quickstart.cpp \
  target/debug/librnet.a -lpthread -ldl -lm -o /tmp/rnet-cpp-game-quickstart
```

Release packages include both example source and support header; from their root use
`lib/librnet.a` instead of `target/debug/librnet.a`. `make cpp-test` also runs the example's
boundary and CLI tests using Python 3 (standard library only).

### Go

The [Go quickstart](../examples/go/game_quickstart/runtime.go) uses two independent
`GameRuntime` instances: pinned identity, listen/connect, server authorization,
`GameSessionReady`, opaque binary/empty echo, logger injection and explicit close.
Transport is chosen at endpoint creation, never per send. Run from the repository root:

```sh
make go-test
./target/rnet-go-game-quickstart tcp --one-slot
./target/rnet-go-game-quickstart udp
./target/rnet-go-game-quickstart kcp --deny  # Expected exit 1, AuthRejected (-13)
```

No arguments defaults to KCP. Go 1.22+ and CGO are required; `make go-test` refreshes
`lib/librnet.a`, runs SDK and example tests, and builds the executable. After changing
native code, use `-a`: Go's cache does not track external static-archive changes.
Release packages include the SDK, archive, `go.mod` and example sources; build from
the package root without a Rust rebuild:

```sh
CGO_ENABLED=1 go build -buildvcs=false -a -p 1 -o rnet-go-game-quickstart ./examples/go/game_quickstart
./rnet-go-game-quickstart tcp --one-slot
```

`-buildvcs=false` avoids consulting an unrelated enclosing Git checkout when
building the source package, which does not ship repository metadata.

`Poll` copies event payloads into Go-owned slices before releasing native tokens,
including cleanup of a partially exported prefix on failure. The example's
[application retry queue](../examples/go/game_quickstart/pending.go) keeps at most
two copies per peer, each at most 128 bytes (empty messages count). `WouldBlock`
retains the head and resumes positive-capacity polls; only successful admission
removes it. The ten-second exchange deadline bounds retry attempts, not native
calls or a blocked user logger. Real single-message scheduler budgets prove actual
backpressure recovery in TCP/UDP/KCP tests; these are not load/soak measurements.
Raw UDP remains unreliable: missing, duplicated or reordered echoes can fail this
strict demo. Production needs its own bounded per-player overload/delivery policy.

Failures preserve `StatusError.Code` through `errors.As`; protocol violations with
zero event status still fail, and the conservative example treats all endpoint
errors as fatal. Cleanup always attempts both runtimes, with separate five-second
backpressure deadlines and `errors.Join` preserving primary and secondary errors.
Quiesce other callers before closing; Go has no finalizer-based cleanup guarantee.

The injected `log.Logger` serializes both native workers' callbacks. It prints
lifecycle diagnostics, not keys/tickets/payloads; use a fast bounded production sink
and monitor dropped logs. The default poll-driven 30-second latency summary includes
P95/P99 and sample counts, but a short echo usually finishes before it appears.
Never stop/close a runtime in its callback. Demo keys/tickets are not a login system:
persist server identity, distribute pins securely, validate credentials and bind
authorization to the authenticated client key. The example clears owned private-key
buffers, but Go value/compiler copies prevent a guarantee of erasing every copy.

See [`game_test.go`](../go/rnet/game_test.go) for additional security transitions,
resume and version-negotiation regressions; the quickstart keeps encryption enabled.

Public methods, constants and fields have GoDoc contracts, including units, zero
values, payload ownership and asynchronous completion. From the repository root,
use `go doc ./go/rnet GameRuntime.Poll` or `go doc ./go/rnet GameConfig` for details.
`make go-test` checks public documentation presence as well as runtime regressions;
presence checks do not prove the comments are semantically correct.

The zero `GameConfig` selects defaults, but a supplied transport `Config` must start
from `DefaultConfig()`. Positive game expiry/log-summary durations round up to
milliseconds; heartbeat configuration and poll waits truncate to milliseconds.
Game `Poll(0, ...)` still drives maintenance but does not drain application events.
Always close runtimes explicitly after stopping concurrent callers, and never call
`Stop`/`Close` from their logger callbacks. Clear your own credential copies.

Clock offsets arrive from native as signed microseconds; Go exposes them as
nanosecond `time.Duration`. Values outside that narrower range saturate to the
signed whole-microsecond bounds, preserving direction instead of overflowing.
`Available=false` means no sample, not an invalid session: pending/stale handles
return an error. Even an authenticated clock sample is approximate and peer-reported,
not an anti-cheat authority. `make go-test` includes Linux-only, explicitly tagged
ABI-boundary regressions for extreme offsets and partial poll exports; those hooks
are absent from normal SDK builds.

### C ABI

[`game_echo.c`](../examples/c/game_echo.c) is a complete C11 game client/server example with two independent runtimes, pinned server identity, application authorization, binary and empty payload echoes, logging, bounded send retries and explicit shutdown. It uses the same API for all three transports:

```sh
make c-test                     # Build, ownership failures, and all transport scenarios
./target/rnet-c-game-echo tcp
./target/rnet-c-game-echo udp
./target/rnet-c-game-echo kcp
./target/rnet-c-game-echo tcp --deny  # Expected nonzero exit with auth rejection (-13)
```

For a manual Linux build from the repository root:

```sh
cargo build -p rnet-ffi
cc -std=c11 -Wall -Wextra -Werror -Iinclude examples/c/game_echo.c \
  target/debug/librnet.a -lpthread -ldl -lm -o /tmp/rnet-c-game-echo
/tmp/rnet-c-game-echo tcp
```

Release packages include `examples/c/game_echo.c` and `game_poll.h`; from the package root use `lib/librnet.a` instead of `target/debug/librnet.a`, without rebuilding Rust.

The C ABI borrows send bytes only for the duration of `rnet_game_send`. Poll's `out_count` describes an owned prefix **even on failure**. Release both nonzero tokens of every event in that prefix, including events not processed because an earlier handler failed; never inspect or release slots beyond it. Copy the first thread-local diagnostic before cleanup overwrites it. The example's [`game_poll.h`](../examples/c/game_poll.h) implements this pattern and is covered by real native buffer ownership tests; it is example code, not an extra SDK interface.

The example retains one bounded payload copy per peer when `send` reports `WouldBlock`. It retries once per loop while both peers continue positive-capacity polling, with a ten-second exchange deadline. It does not retain native event pointers or grow an unbounded retry list. A real multi-player application needs its own bounded per-session admission and overload policy. This demo allows one in-flight request, does not retransmit lost UDP application packets, and treats unexpected duplicates or payloads as a demo failure.

Never deploy the literal demo ticket or trust keys received over an untrusted connection. The server must validate credentials and bind them to the authenticated client key; use persistent server identity and a trusted public-key distribution channel. The default example keeps encryption enabled. Runtime stop/destroy runs outside logger callbacks, releases exported tokens before destruction, and retries stop backpressure with a separate deadline. A successful send/stop is not proof of remote delivery; the echo is the application acknowledgement in this example.

The logger prints lifecycle diagnostics to stderr for demonstration, without payload or ticket bytes. In production adapt the callback to a fast sink, keep its context alive through destroy, and monitor dropped logs. The default 30-second poll-driven `game_latency_summary` includes sample counts and P95/P99; a sub-second echo normally finishes before that summary is emitted.

### C# (.NET 8+)

See the [C# guide](csharp.md) and [standalone quickstart](../csharp/RNet.Quickstart/GameExample.cs) for two runtimes, binary/empty echo, authorization, bounded retries, logging and explicit Stop/Dispose. `make csharp-test` runs its boundary/CLI tests and the existing [`RNet.Smoke`](../csharp/RNet.Smoke/Program.cs) regressions. `--one-slot` exercises real scheduler backpressure; `--deny` intentionally fails login. `Poll` returns copied managed payloads; retain those when `Send` throws `RNetException` with `Code == -4` and continue positive-capacity polling. Explicit Stop exposes stop errors; Dispose does not report SafeHandle's internal native teardown failures. Quiesce callers and do not stop/dispose from a logger callback. Current validated scope remains Linux x86_64/.NET 8, not Unity/IL2CPP or other platforms.

For every language binding, keep exactly one logical poll dispatcher per runtime and route copied events to gameplay workers. Sends may originate from application workers, but business code must define how it queues and retries `WouldBlock` without blocking the poll dispatcher.

A unified TCP client endpoint belongs to one connection: it is reclaimed when that connection's I/O worker exits, including after remote disconnect or local `close_session`. A server listener is shared and remains open when one accepted session closes. Local `close_session` invalidates the session immediately; worker/endpoint cleanup is asynchronous, so use `close_endpoint` when explicit endpoint cancellation is required. An already-reclaimed endpoint returns `InvalidHandle` if closed again. Reconnect creates a new endpoint; keep polling so the game facade also releases its cached state. A session close does not introduce an extra `EndpointError`; legacy transport endpoints have been removed.

`send_latest(session, key, payload)` and `send_latest_with_tick(...)` stage replaceable snapshots in a separate bounded queue. Keys are scoped to one session; replacing a pending key releases its previous budget and preserves fair key order. The queue flushes on `poll`, or the game tick can call `flush_realtime(capacity)` explicitly. If the transport queue is full, the pending key rotates behind others instead of blocking the event loop. `RealtimeQueueConfig` caps total bytes, per-session bytes, keys per session, and each flush batch. `realtime_queue_snapshot()` and Prometheus expose queued bytes/messages, game-stage replacements, close/backpressure drops, send failures, and forwarded totals. For TCP/KCP, a keyed slot can also be replaced after transport admission while it is still waiting for the I/O worker; `transport_latest_snapshot()` reports those replacements, worker pickups and admission failures split by stable reason. Worker pickup is the recall boundary: a partly written TCP record or data already given to KCP cannot be withdrawn, including KCP's own deferred/retransmission queues. A pickup does **not** prove delivery; a later I/O failure is traced through the existing session-close/error metrics. UDP retains game-stage-only replacement. Use ordinary `send` for messages that must not be dropped or replaced. A successful forward means queued locally, **not delivered**.

## Single-runtime reconnect

After ordinary authorization and `SessionReady`, the server may call `issue_resume_ticket(server_session, identity)` with an opaque 1–64-byte application identity. The client receives `GameEvent::ResumeTicket`; keep its bytes private and call `connect_resume(config, old_client_session, ticket.as_bytes())` or `connect_host_resume(...)` after interruption. These methods perform a **new Noise handshake** and use an explicit `RGR3` resume join; ordinary joins use `RGV3`. The same client static key, listener, protocol ID, and version are required. Old wire-v2 markers are rejected.

The server receives `GameEvent::ResumeRequest` containing the claimed old server handle and identity, the new session, and the new join proof. It must recheck application identity and call `auth_decide(new_session, true)` or `false`. Only success emits `SessionResumed { old_session, new_session }` on each side; this event means the new handle is ready, and the old handle is invalid. Authorization alone does not invalidate a still-live old route: takeover waits for the protected SELECT/ACK/READY exchange, and negotiation failure leaves the old route usable. A denied request never changes the old authorized session. Explicit `close_session`/`close_endpoint` revokes applicable tickets and pending claims. Tickets are valid once for 30 seconds by default; `with_resume(ttl, max_tickets)` changes the bounded policy (maximum TTL: five minutes). `resume_metrics_snapshot()` and `rnet_game_resume_*` Prometheus metrics expose issued, received, rejected, denied, revoked, resumed, and outstanding counts without player labels.

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

Established unified TCP failures additionally preserve a stable diagnostic phase (`tcp_read`,
`tcp_framing`, `tcp_decode`, `tcp_inbound`, `tcp_write`, `tcp_security_write`,
`tcp_rekey_write`, or `tcp_security_transition`) and the available local cause in the
`game_session_closed` log. EOF is explicitly identified as peer closure rather than invented
OS error text. Detail is bounded to 1024 UTF-8 bytes and never includes peer payload/credentials.
The transport close/join-failure event's diagnostic bytes are optional: under event-byte
pressure they are discarded before they could cost lifecycle delivery or evict extra messages.
The original status and handles remain authoritative; event-count exhaustion still follows the
existing lifecycle rejection policy. Game event structs and wire formats are unchanged.

With a logger configured, polling also emits `game_latency_summary` every 30 seconds by default. It includes cumulative count/P90/P95/P99/P99.9/max (microseconds) for heartbeat RTT, transport phases, scheduling and logger callbacks, plus queue gauges and logger failures. Zero samples are not evidence of zero latency. Change the period with `set_metrics_log_interval(Duration)` in Rust, `rnet_game_metrics_log_interval_set(runtime, milliseconds)` in C, `set_metrics_log_interval(milliseconds)` in C++11, or `SetMetricsLogInterval(time.Duration)` in Go; zero disables the summary. Lifecycle errors preserve available library-generated causes, but credentials and application bytes never become diagnostics. Already-established close events may contain only their stable reason code.

For adaptive TCP/UDP/KCP failures, `game_session_closed` includes an available local `phase=...` and cause alongside the endpoint, session and stable error code. Useful datagram phases are:

| Phase | Meaning |
| --- | --- |
| `datagram_idle` | No successful received or queued wire activity before the idle deadline |
| `datagram_security_transition` | The server-led security barrier did not receive its acknowledgement in time |
| `datagram_handshake` / `datagram_auth_wait` | Handshake failure, or waiting for application authorization / its response |
| `datagram_auth_write` / `datagram_security_write` | Failed to encode or send a protected control record |
| `datagram_control_retry` | Raw UDP control retransmission exhausted its retry limit |
| `datagram_encode` / `datagram_write` | Immediate outbound encoding or wire submission failed |
| `datagram_deferred_encode` / `datagram_deferred_write` | A queued outbound message failed when released after a barrier or backpressure |
| `datagram_receive` / `datagram_cookie` | Fatal receive failure or local cookie initialization failure; the owning endpoint is invalidated |
| `datagram_connect` / `datagram_hello` / `datagram_preflight` | Terminal client deadline, initial hello or preflight failure; the client endpoint is invalidated |
| `datagram_rebind` / `datagram_rebind_address` / `datagram_retarget_endpoint` / `datagram_retarget_session` | Terminal local failure while switching connection candidates |
| `tcp_accept` | Fatal unified TCP listener exit; the endpoint and every session it owns are closed |

These are diagnostic labels, not a new wire or business API: branch on typed error codes, not the free-form text. Close details are local-only, at most 1024 UTF-8 bytes, and optional under event-byte pressure. `WouldBlock` for queued business data remains backpressure, not a forced disconnect. Invalid incoming datagrams are counted and discarded without per-packet logging or disconnecting established peers. KCP's later socket flushes may retry transient failures without a close log; a successful send submission is not proof of remote delivery.

When an adaptive UDP/KCP endpoint terminates on a receive failure, the endpoint and its sessions become invalid; other endpoints remain usable. The runtime attempts to publish `EndpointError` followed by session failures/closures with the local cause. A pending client also receives `JoinFailed` before its close, if the event queue admits these notifications. Cleanup and resource-budget release do not depend on event delivery. Polling (including capacity zero) reconciles invalid endpoint ownership in the game facade, clearing protocol policies, pending authorization, heartbeat/clock state, staged sends and endpoint-scoped resume tickets even if notifications were lost. There is no automatic reconnect. A later explicit close of an invalid handle returns `InvalidHandle`.

Not every `EndpointError` is fatal: address failover and some TCP errors retain a valid endpoint. Do not treat the event kind alone as a disconnect instruction; query endpoint validity. This change preserves the existing receive-error exit policy, rather than introducing a new cross-platform ICMP/retry classification. The healthy polling path compares endpoint counts before scanning policies, and does not scan every endpoint on each poll.

Unified UDP/KCP client setup also invalidates its endpoint on terminal failure, including the last candidate's timeout and failure to publish the opening event. Repeated failed attempts therefore release endpoint capacity without an extra explicit close. Candidate failover itself retains the endpoint. If overload has already closed a session, endpoint cleanup does not invent a second close or guarantee a lost join notification.

A fatal unified TCP accept error invalidates the listener and all its pending/established sessions, and cancels their socket tasks, including silent handshakes. Other listeners and their sessions remain usable. Normal `stop(drain_timeout)` is not this failure path: its grace period is retained until explicit endpoint closure. Cleanup is independent of bounded event delivery, and late handshake errors cannot duplicate a closure already won by endpoint cleanup. An accept error does not imply a process crash; a process that has actually terminated cannot publish cleanup events. This contract does not introduce automatic relistening/reconnect. Legacy transport endpoints have been removed.

The [C# SDK guide](csharp.md) covers the same game lifecycle through .NET 8 on Linux x64, including complete send/receive loops, injected loggers, dynamic encryption and managed/native ownership. Its `SetMetricsLogInterval(milliseconds)` and `PrometheusSnapshot()` require the matching native library from the same package.

All game joins use current `RGV4`/`RGR4` markers and reject previous SDK wire formats, including `RGV3`/`RGR3`. During v4's final confirmation, early business data is held behind the ready event with per-session limits and a runtime-wide count/byte budget derived from `event_queue_capacity` and `max_event_bytes`; its reservation remains charged until application polling or session cleanup. `range_buffer_snapshot()` reports current/peak/limit gauges and per-session/runtime rejection totals, and the same low-cardinality series use the `rnet_game_range_early_*` Prometheus prefix. Raw UDP business messages carry a network-owned sequence extension separate from the optional application sequence. `udp_loss_snapshot(session)` reports receiver-side gaps within a 64-packet reorder window; a failed local send does not consume a sequence. `kcp_retransmission_snapshot(session)` reports per-session PUSH retransmissions independently of heartbeat availability. `network_quality(session)` includes `grade`, `basis`, optional `udp_loss`, and optional `kcp_retransmissions`; `QualityPolicy` can tune grade thresholds. KCP counts PUSH segments sent again before acknowledgement and uses a rolling 128-segment window. `GameEvent::QualityChanged` requires two consecutive non-unknown samples at a changed grade/basis and suppresses repeats. `basis=LatencyOnly` means no transport loss signal was used (including TCP). Neither UDP gaps nor KCP retransmissions are an IP-layer packet capture, and low sample counts do not establish a reliable loss rate. Heartbeat samples remain authenticated even if business records are plaintext, but UDP business-data sequences in that mode are **not integrity-protected** and must be treated as advisory rather than a trusted security or billing signal. Clients automatically exchange protected four-timestamp `ClockSync` controls after game readiness. `clock_sync_snapshot(session)` yields an optional server-minus-client offset and network RTT; `clock_micros()` gives the runtime-local monotonic origin. The sample uses process-local monotonic clocks and includes local/network queuing, so it is only an approximate transport sample, never UTC or a trusted anti-cheat time source. Rust returns `None` without a sample; C uses `rnet_game_clock_sync_t.available` as the has-sample bit (`0` on server sessions), with equivalent `available`/`Available` fields in C++11/Go. Tick-annotated local scheduler delay is reported separately from clock/network RTT and must not be interpreted as anti-cheat time.

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

With `GameRuntimeConfig::production()`, business plaintext is disabled. A deployment that explicitly accepts the risk can use `GameRuntimeConfig::production().allow_plaintext_business_data(true)` without bypassing authenticated connection setup or business authorization. The server can then call `runtime.set_encryption(session, false)` and later `runtime.set_encryption(session, true)`; clients follow authenticated security-control messages automatically. Plaintext business records genuinely lack confidentiality and integrity: an on-path party can alter valid application payloads. A malformed envelope or forged control kind from the unverified plaintext business channel is silently dropped and increments `rnet_game_plaintext_invalid_envelopes_dropped_total`; it does not produce a per-packet log/event that an attacker could amplify into the game loop. The same error in a Noise-protected business record still closes the session and publishes `ProtocolViolation`, even after a live mode switch. Authenticated-control errors also close the session. Corrupting outer record framing can still cause a transport-level disconnect, so plaintext provides no availability or integrity guarantee. Avoid plaintext on public networks. Authentication and mode-switch control stay protected by Noise.

The server may call `runtime.rekey(session)` to rotate established session keys without changing whether business data is encrypted. Clients cannot initiate a rekey. `SecurityChanged` reports the completed operation and security epoch; the application continues using the same session handle and `send` API.

`GameProtocol::new(id, min, max)` is the single application protocol configuration for `listen`, `connect` / `connect_host`, and `connect_resume` / `connect_host_resume`. Bounds are inclusive and nonzero; equal bounds require an exact version. There are no separate range types or entry points. The server chooses the highest version in the intersection before business authorization and authenticates its choice with a session-bound nonce, protocol ID, and Noise-protected SELECT/ACK/READY exchange. `selected_protocol_version(session)` is available to the server during `AuthRequest` and to the client after selection. No overlap is rejected before business authorization; previous SDK wire formats are rejected. A resumed session keeps the ticket's original selected version, gets a new handle and requires new business authorization; the new client range must still include that version. `build_id` and `capabilities` remain authenticated metadata for application authorization, not permission to activate network-library features. See [game_range.rs](../crates/rnet-game/tests/game_range.rs) for TCP/UDP/KCP and resume cases.

The complete live-session test in [game_runtime.rs](../crates/rnet-game/tests/game_runtime.rs) covers TCP, UDP, KCP, plaintext-to-encrypted-to-plaintext transitions, rekey, hostname joining, and version rejection.

## C, C++11, and Go basic game APIs

The `rnet_game_*` C functions use separate game runtime handles; do not pass them to `rnet_runtime_*` transport functions. Check `rnet_abi_version() == RNET_ABI_VERSION` before using the ABI; the current discriminator is 4 and older layouts are not supported. Use `rnet_game_config_init`, then `rnet_game_runtime_create` with optional `rnet_client_security_t` for a pinned client identity. A server uses `rnet_game_server_listen` with `rnet_game_server_config_t`: transport, protocol ID, min/max versions, key, and initial encryption. A client uses `rnet_game_client_connect` with `rnet_game_client_config_t`: transport, hostname/IP, protocol ID, min/max versions, and opaque join ticket; it does not choose encryption. Equal bounds require an exact version; `rnet_game_selected_protocol_version` returns the server-selected version. After `RNET_GAME_AUTH_REQUEST`, the server calls `rnet_game_auth_decide`; only game-ready handles may send using `rnet_game_send(runtime, session, payload)`. Poll with `rnet_game_poll_events`, and release both nonzero event buffer tokens with `rnet_game_buffer_release` before destroying the runtime. Credential buffers are erased by the C layer when released; application-side copies remain the application's responsibility. For same-runtime reconnection, a ready server session calls `rnet_game_issue_resume_ticket`, the client receives `RNET_GAME_RESUME_TICKET`, then calls `rnet_game_client_resume_connect` with its old local handle. The server must authorize `RNET_GAME_RESUME_REQUEST` again; after negotiation each side receives `RNET_GAME_SESSION_RESUMED` with its own old handle in `related_session`.

C++11 users can include `rnet.hpp` and construct `rnet::GameRuntime` with a client key and pinned server key, then call `listen`, `connect`, `auth_decide`, `send`, and `poll`. [`game_echo_smoke.cpp`](../examples/cpp/game_echo_smoke.cpp) is a runnable TCP example; the same methods work for UDP and KCP by changing only `GameServerOptions.transport` and `GameClientOptions.transport`. Both configs use the `GameProtocol` fields `id`, `min_version`, and `max_version`, including `connect_resume`; `selected_protocol_version` exposes the result. See [`game_range_smoke.cpp`](../examples/cpp/game_range_smoke.cpp) for unequal bounds. `GameRuntime::poll` copies event bytes into owned vectors and releases native tokens even if a C++ allocation throws.

Go users can call `NewGameRuntime(&ClientSecurity{LocalKey: clientKey, ExpectedServerPublicKey: serverKey.Public[:]})`, then `Listen(GameServerConfig{...})`, `Connect(GameClientConfig{...})`, `AuthDecide`, `Send`, and `Poll`. [`game_test.go`](../go/rnet/game_test.go) shows authorization, opaque-payload exchange, and one-use recovery for TCP, UDP, and KCP. Both configs use `GameProtocol{ID: id, MinVersion: min, MaxVersion: max}`, including `ConnectResume`; `SelectedProtocolVersion` exposes the result. Go `Poll` copies native event data and returns all C tokens before returning. `GameEvent.Data` may contain a join credential or resume ticket; default `%v`/`%#v` formatting of game events and configs redacts those bytes, but do not log the byte slices explicitly and erase your own copies when appropriate. Go's zero-value `GameServerConfig.InitialSecurity` means encrypted; choose `SecurityPlaintext` explicitly and enable `GameConfig.AllowPlaintextBusinessData` only when required.

This cross-language slice covers the ordinary and range-version game flows, server encryption switching, session/endpoint close, rekey, `LatestOnly` staging, single-runtime resume, event metadata, per-session quality snapshots, runtime-wide real-time queue counters, cumulative heartbeat/resume/clock/logger metrics, optional bounded structured game-event logging, and a no-player-label Prometheus snapshot. `rnet_game_network_quality` reports `available=0` before the first authenticated RTT sample; its separate availability flags distinguish TCP's unavailable loss from UDP sequence gaps and KCP retransmission ratios. `rnet_game_realtime_queue_snapshot` (C), `realtime_queue_snapshot()` (C++11), and `RealtimeQueueSnapshot()` (Go) report current staged-message/byte gauges plus cumulative admission-rejected, replaced, closed-dropped, backpressure-dropped, send-failed, and forwarded counts. `rnet_game_range_buffer_snapshot` (C), `range_buffer_snapshot()` (C++11), and `RangeBufferSnapshot()` (Go) report the wire-v4 early-data current/peak/limit gauges and per-session/runtime rejection totals. `rnet_game_transport_latest_snapshot` (C), `transport_latest_snapshot()` (C++11), and `TransportLatestSnapshot()` (Go) report TCP/KCP pending replacements, irreversible worker pickups, and admission failures split into would-block, invalid-handle/state, handshake-required, unsupported, too-large and other. The older replacement-only queries remain available. Prometheus exports the same cumulative counters. Forwarded and pickup mean local handoff, **not delivery**. `rnet_game_metrics_snapshot` (C), `metrics_snapshot()` (C++11), and `MetricsSnapshot()` (Go) expose cumulative game counters, RTT percentiles and a `logger_available` flag; callback latency is measured on the asynchronous logger thread. These runtime-wide metrics have no per-player labels and require no buffer release. Go callback panics cannot cross the C boundary; the Go facade recovers them and adds them to `MetricsSnapshot().Logger.SinkPanics` and a separate `rnet_game_go_logger_panics_total` Prometheus counter. Release the Prometheus C buffer token after reading. For capacity tuning, set `rnet_game_config_t.network_config` to a borrowed `rnet_config_t` while creating the C runtime; C++11 has a `GameRuntime(config, ...)` constructor, and Go accepts `GameConfig{Network: &network}` where `network` starts from `DefaultConfig()`. The game-specific plaintext flag remains authoritative; unauthenticated endpoints are no longer available, and transport-level logger callbacks are rejected. Configure a game logger at creation with `rnet_game_config_t.logger` and `rnet_game_runtime_create`, the C++11 constructor accepting `rnet_logger_t`, or Go `GameConfig{Logger: callback}`. The callback may query metrics but must not stop/destroy its runtime; its user data must stay valid until destroy returns. It must return promptly, because destroy waits for the bounded dispatcher to finish. Do not substitute the old transport API on a game runtime handle.

For advanced sends, C uses `rnet_game_send_ex` plus `rnet_game_poll_events`; C++11 and Go use `GameSendOptions` and expose the returned correlation ID. `rnet_game_scheduled_queue_snapshot` and its C++11/Go wrappers expose per-priority and queue-delay telemetry. Queue budgets are configured through the single complete `rnet_game_config_t` and `rnet_game_runtime_create`. C++11 accepts that same layout; Go exposes `RealtimeQueue` and `ScheduledQueue` fields on `GameConfig`. There is one game event layout, including correlation metadata, and one poll entry point.

## Current production limits

Current KCP connections negotiate a fresh random conversation during stateless cookie preflight, so delayed packets from a previous connection at the same address cannot enter a new engine. The pinned KCP dependency is compiled with wrapping arithmetic only for that package, matching KCP's modulo-2^32 serial semantics in debug, test and release builds; RNet itself keeps overflow checks. Boundary and stale-conversation regressions are included. The legacy fixed-conversation secure KCP implementation and its constructors have been removed; use transport-neutral `listen`/`connect`. Historical short AddressSanitizer campaigns and Miri results are recorded in the [qualification record](game-production-qualification.md), not a certification of the current tree. They do not replace renewed release gates, long fuzzing, target-environment weak-network soak, or independent Noise/KCP review.
