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
- Persist private keys in an external keystore, pin or verify the server public key, and rotate identities according to the application's incident policy. Never log key, cookie, ticket, or payload bytes.
- Size event and send byte budgets from a process RSS limit. Alert before gauges remain above 80% of their configured limits.
- Size KCP unacknowledged-data budgets for the loss envelope. Application queue bytes and KCP retransmission bytes are independently bounded layers, so include both when deriving the process RSS limit; reserved control capacity prevents data saturation from blocking protocol progress.
- Leave TCP_NODELAY enabled for interactive traffic. Treat requested socket buffer sizes as kernel hints and verify effective values and host `sysctl` ceilings during qualification.
- Put Internet-facing listeners behind ingress filtering and volumetric DDoS controls. Cookie preflight and per-prefix admission protect process state; they do not absorb link saturation.
- Host names and textual IPv4/IPv6 addresses are accepted. TCP, UDP, and KCP retry resolved candidates under one overall deadline. UDP/KCP rebind their client socket when a later candidate changes address family and update the endpoint/session route atomically.

## Observability gate

Export `metrics_snapshot_v3` and rotate latency windows at the monitoring interval. At minimum alert on event drops, lifecycle rejection, admission rejection by reason, endpoint/session/pending-handshake gauges, protocol errors, close reason, send backpressure, queued bytes, KCP update delay, and P99/P99.9. Use logger v2 to retain the timestamp, event name, runtime, endpoint, session, transport, error code, and correlation ID needed to join an incident timeline.

For Rust game deployments, also scrape `GameRuntime::prometheus_snapshot()` or sample `heartbeat_metrics_snapshot()`. Track heartbeat timeouts, rejected replies, probe/reply send failures, and RTT P95/P99 together; the percentile values are zero until `rtt_samples_total` is nonzero. For incident-sensitive RTT percentiles, call `drain_heartbeat_rtt_window()` at each monitoring interval; it does not reset cumulative data. These are runtime-wide aggregates, not per-player labels. The per-session `network_quality(session)` snapshot is intended for targeted diagnosis and is removed when the session closes.

Runtime/session/endpoint failure log records are emitted when the corresponding event is polled. Production consumers must continuously drain events; an application that stops polling also stops advancing these event-derived log records and applies backpressure to the bounded event queue.

## Capacity and soak

Run one transport at a time so results have an unambiguous resource profile:

```sh
./scripts/run-soak.sh tcp 86400 4096 /var/tmp/rnet-soak
./scripts/run-soak.sh udp 86400 1024 /var/tmp/rnet-soak
./scripts/run-soak.sh kcp 86400 4096 /var/tmp/rnet-soak
```

Repeat with production-sized connection counts and the application consumer. Add a `tc netem` matrix covering expected and failure-envelope RTT, jitter, loss, duplication, and reordering. A release passes only if its documented SLO is met, RSS reaches a stable plateau, queues recover after bursts, no close reason is unexplained, and no peer starves another. Archive the raw report with kernel, CPU, memory, toolchain, commit/package hash, and configuration.

## Remaining external assurance

The repository test suite is not a formal cryptographic certification or an independent audit of `snow`, `ring`, KCP, the complete protocol integration, key management, or the deployment environment. High-value deployments require an independent security review and incident-response exercise before exposure.
