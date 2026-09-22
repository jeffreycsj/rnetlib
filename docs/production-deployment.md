# Production deployment gate

RNet supplies bounded runtime behavior and production-test tooling; capacity is accepted only after it is measured on the target kernel, hardware, network, and application event loop.

## Required release checks

Run these from a clean source package:

```sh
make check
cargo check --locked --manifest-path fuzz/Cargo.toml --bins
```

CI additionally runs RustSec, cargo-deny, six libFuzzer smoke campaigns, and an optional Miri job. Do not waive an advisory, license, yanked crate, or unknown source without a dated owner and expiry recorded in `deny.toml` or the security review.

## Runtime policy

- Start new C/C++/Go deployments with config v5; Rust starts with `RuntimeConfig::production()`. Set explicit runtime-wide endpoint and pending-handshake limits instead of relying only on per-listener capacity.
- Keep plaintext business data and legacy unauthenticated endpoints disabled unless a reviewed compatibility exception requires them.
- Continuously poll every game runtime. Heartbeat scheduling and authenticated reply handling are poll-driven; a stalled game event loop cannot make progress on liveness or per-session quality.
- `send_latest` is a best-effort pre-transport coalescing path for replaceable snapshots. Poll continuously or explicitly call `flush_realtime`; monitor admission rejection, replacement, close/backpressure drops, send failures, and queued bytes. Keep commands that require reliable delivery on ordinary `send`.
- Ordinary and advanced game sends use the bounded fair scheduler. Tune Rust `ScheduledQueueConfig`, C `rnet_game_config_v2_t`, C++11's V2 constructor, or Go `GameConfig.ScheduledQueue`; monitor per-priority admission/forwarding, backpressure requeues, expiry and queue-delay percentiles. An expiry is local and cannot recall socket/KCP-owned bytes.
- Resume tickets are single-runtime and one-use; a server restart or a different instance cannot recover the identity. Keep the same client Noise static key, reauthorize every `ResumeRequest`, monitor `rnet_game_resume_*`, and revoke tickets on an application kick. Do not advertise rolling-restart recovery until a shared atomic ticket store is implemented and tested.
- Persist private keys in an external keystore, pin or verify the server public key, and rotate identities according to the application's incident policy. Never log key, cookie, ticket, or payload bytes.
- Size event and send byte budgets from a process RSS limit. Alert before gauges remain above 80% of their configured limits.
- A wire-v4 game runtime uses `event_queue_capacity` and `max_event_bytes` again as an independent ceiling for authenticated business data that arrives before the ready event. Include both the transport event queue and this game-layer early-data allowance when calculating worst-case RSS; reservations survive negotiation completion until the application polls them.
- Size KCP unacknowledged-data budgets for the loss envelope. Application queue bytes and KCP retransmission bytes are independently bounded layers, so include both when deriving the process RSS limit; reserved control capacity prevents data saturation from blocking protocol progress.
- Leave TCP_NODELAY enabled for interactive traffic. Treat requested socket buffer sizes as kernel hints and verify effective values and host `sysctl` ceilings during qualification.
- Put Internet-facing listeners behind ingress filtering and volumetric DDoS controls. Cookie preflight and per-prefix admission protect process state; they do not absorb link saturation.
- Host names and textual IPv4/IPv6 addresses are accepted. TCP, UDP, and KCP retry resolved candidates under one overall deadline. UDP/KCP rebind their client socket when a later candidate changes address family and update the endpoint/session route atomically.

## Observability gate

Export `metrics_snapshot_v3` and rotate latency windows at the monitoring interval. At minimum alert on event drops, lifecycle rejection, admission rejection by reason, endpoint/session/pending-handshake gauges, protocol errors, close reason, send backpressure, queued bytes, KCP update delay, and P99/P99.9. Use logger v2 to retain the timestamp, event name, runtime, endpoint, session, transport, error code, and correlation ID needed to join an incident timeline.

For Rust game deployments, also scrape `GameRuntime::prometheus_snapshot()` or sample `heartbeat_metrics_snapshot()`. Track heartbeat timeouts, rejected replies, probe/reply send failures, RTT P95/P99, scheduled queue delay P95/P99, tick-annotated queue delay and expiry together; percentile values are zero until their sample counter is nonzero. For incident-sensitive RTT percentiles, call `drain_heartbeat_rtt_window()` at each monitoring interval; it does not reset cumulative data. These are runtime-wide aggregates, not per-player or correlation labels. The per-session `network_quality(session)` snapshot is intended for targeted diagnosis and is removed when the session closes. A failed deferred send emits the bounded `game_scheduled_send_failed` log with its correlation ID and no payload.

Runtime/session/endpoint failure log records are emitted when the corresponding event is polled. Production consumers must continuously drain events; an application that stops polling also stops advancing these event-derived log records and applies backpressure to the bounded event queue.

## Capacity and soak

Run one transport at a time so results have an unambiguous resource profile:

```sh
./scripts/run-soak.sh tcp 86400 4096 /var/tmp/rnet-soak
./scripts/run-soak.sh udp 86400 1024 /var/tmp/rnet-soak
./scripts/run-soak.sh kcp 86400 4096 /var/tmp/rnet-soak
```

The transport probe above does not exercise the game facade. For a local game-layer baseline,
run each transport with the intended duration, client count, payload bytes, and send rate per
client (the examples below run 24 hours):

```sh
./scripts/run-game-soak.sh tcp 86400 128 256 20 /var/tmp/rnet-game-soak
./scripts/run-game-soak.sh udp 86400 128 256 20 /var/tmp/rnet-game-soak
./scripts/run-game-soak.sh kcp 86400 128 256 20 /var/tmp/rnet-game-soak
```

The game probe reports completion, successful sends/receives, backpressure, heartbeat RTT P95/P99,
timeouts, protocol/event errors, process CPU, and RSS every 60 seconds. It keeps client and server
endpoints in one process on loopback, so it is a reproducible facade regression and local soak,
not a substitute for a distributed target-environment capacity test. An exit status of zero means
the probe completed without an unexpected session failure; apply deployment-specific SLOs to the
report before accepting a release. Reports mark whether the source tree was dirty; a commit ID from
a dirty run does not identify the exact tested source. The script refuses to start below 15 GiB
`MemAvailable`. The `game_soak` cargo smoke test treats only that exact resource-guard refusal as
an explicit skip on smaller CI runners; a green test run with this skip is **not** soak evidence.

Repeat with production-sized connection counts and the application consumer. Add a `tc netem` matrix covering expected and failure-envelope RTT, jitter, loss, duplication, and reordering. A release passes only if its documented SLO is met, RSS reaches a stable plateau, queues recover after bursts, no close reason is unexplained, and no peer starves another. Archive the raw report with kernel, CPU, memory, toolchain, commit/package hash, and configuration.

## Remaining external assurance

The repository test suite is not a formal cryptographic certification or an independent audit of `snow`, `ring`, KCP, the complete protocol integration, key management, or the deployment environment. High-value deployments require an independent security review and incident-response exercise before exposure.
