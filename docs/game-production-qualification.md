# Game production qualification record

This record separates repository qualification from target-environment certification. It was
updated on 2026-09-22 against the dirty development tree shown by the commands below; rerun every
gate on the release commit and archive the resulting package hash before deployment.

## Repository gates

- Rust formatting, strict workspace Clippy, all workspace targets, structure limits, release ABI
  exports, C++11 final-link smoke tests, Go/cgo tests, and Go race tests pass.
- The Linux x86_64 SDK package was regenerated with its SHA256 manifest. Both shipped C++11 game
  examples compile against the packaged headers/static library and complete their loopback smoke
  flows. It also includes a prebuilt, hash-reporting game-soak probe and runner that works without
  the source tree, plus the game guide and this qualification record.
- All six fuzz targets (`control_parser`, `datagram_preflight`, `ffi_config`, `frame_parser`,
  `game_wire`, and `kcp_engine`) completed five-second, one-job, 1 GiB RSS-limited smoke campaigns
  with release arithmetic and no sanitizer. A newly found impossible future KCP ACK timestamp was
  preserved as a regression and is rejected before entering the pinned dependency.
- `cargo audit` scanned 104 locked dependencies without a reported advisory. `cargo deny check`
  passed advisories, licenses, bans, and sources; the known duplicate `getrandom`, `syn`, and
  `windows-sys` versions remain visible warnings.
- A bidirectional local three-second probe per transport used eight loopback clients, 256-byte
  payloads, and 100 sends per client per second. TCP and UDP each completed 2,248 client → server →
  client messages; KCP completed 2,240. All paths had zero duplicates, reorder/too-old observations,
  backpressure, closes, event drops, or protocol errors. TCP/UDP/KCP respectively observed
  7.0/12.6/7.4% process CPU and 3,588/3,852/3,860 KiB RSS on the recorded 48-core VM. Scheduled
  queue P99.9 was 1,098/1,870/1,099 µs, transport send-queue P99.9 was 1,286/684/2,048 µs, and event
  queue P99.9 was 1,032/1,752/3,064 µs. This is a smoke result, not a capacity claim.
- The soak harness verifies a bidirectional client/server echo, fails incomplete reliable drains,
  emits P99.9/max and bounded-queue/close-reason telemetry, and records the exact release probe and
  lockfile hashes.

## Tooling limits

- Local ASan fuzzing cannot run with the installed stable-only Rust 1.96 toolchain: cargo-fuzz
  requires nightly `-Zsanitizer`, and this host has no `rustup` toolchain manager. CI now installs
  nightly and explicitly runs every fuzz target with AddressSanitizer and a 2 GiB per-process RSS
  limit; non-sanitized local coverage and saved-crash replay also pass.
- Miri is not installed locally. CI installs the nightly Miri component and runs the pure core and
  protocol crates on every push and pull request.
- The production send-admission gate is exercised with Loom across racing send/session-close and
  send/runtime-stop interleavings. `make loom-test` runs the same bounded model locally and in CI;
  deterministic concurrency regressions and Go race coverage remain enabled as separate layers.
- `snow` and the complete Noise/KCP integration have not received an independent third-party
  protocol audit.

## Final external gate

Do not label a target deployment certified until the release build completes the documented
24–72 hour TCP/UDP/KCP soak and `tc netem` loss, latency, jitter, duplication, and reorder matrix on
the actual kernel, hardware, network path, and application poll loop. Archive CPU, RSS, queue
gauges, close reasons, P95/P99/P99.9 latency, packet-quality counters, configuration, toolchain,
and package hash. Cross-instance recovery remains intentionally out of scope.
