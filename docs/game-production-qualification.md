# Game production qualification record

This record separates repository qualification from target-environment certification. It was
updated on 2026-09-23 against the dirty development tree shown by the commands below; rerun every
gate on the release commit and archive the resulting package hash before deployment.

## Repository gates

- The subsequent established-TCP diagnostics increment preserves local close phase/cause without
  changing status codes or the game ABI. Tests cover real EOF, corrupted framing and lost rekey
  acknowledgements, concurrent close winners, bounded UTF-8 allocations, and byte-budget fallback
  that preserves lifecycle identity/status. Final serial workspace/Clippy gates pass, as do the
  three new queue tests under pinned Miri. Go/cgo verification now forces rebuild/test execution
  (`-a -p 1 -count=1`), including race, because Go does not track external native archives in its
  build cache. This change does not replace the final target soak.

- The 2026-09-23 game SDK review added a Linux x86_64 / .NET 8 C# facade and poll-driven game
  latency summaries, and fixed native/Go logger teardown deadlocks and Go native-TLS diagnostics.
  Strict Clippy, serial workspace tests, C++11, Go/race, Loom, ABI checks and C# checks pass locally.
  C# also passes against the actual packaged managed/native binaries, including all three
  transports, live security changes, recovery, version selection, sampled telemetry, wrong-key
  rejection, logger reentry and managed ownership. See [the review](game-library-review.md) for
  remaining limitations. ASan/Miri and short traffic measurements below are retained evidence
  from the preceding qualification, not campaigns repeated for this SDK change.

- Rust formatting, strict workspace Clippy, all workspace targets, structure limits, release ABI
  exports, C++11 final-link smoke tests, Go/cgo tests, and Go race tests pass.
- The Linux x86_64 SDK package was regenerated with its SHA256 manifest. Both shipped C++11 game
  examples compile against the packaged headers/static library and complete their loopback smoke
  flows. It also includes a prebuilt, hash-reporting game-soak probe and runner that works without
  the source tree, plus the game guide and this qualification record.
- All six fuzz targets (`control_parser`, `datagram_preflight`, `ffi_config`, `frame_parser`,
  `game_wire`, and `kcp_engine`) completed five-second, one-job, 1 GiB RSS-limited smoke campaigns
  with release arithmetic and AddressSanitizer on pinned `nightly-2026-09-21` / cargo-fuzz 0.13.2.
  The local sandbox forbids LeakSanitizer's ptrace operation, so this local run used
  `ASAN_OPTIONS=detect_leaks=0`; CI deliberately keeps the default leak check enabled. A newly
  found impossible future KCP ACK timestamp was preserved as a regression and is rejected before
  entering the pinned dependency.
- Miri executed all 26 tests in the pure `rnet-core` and `rnet-protocol` suites on the same pinned
  nightly with no undefined-behavior report. Nightly deprecation diagnostics also identified five
  atomic update calls. The future rename is suppressed at their narrow function boundaries because
  the replacement API would violate the declared Rust 1.85 MSRV.
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

## Tooling qualification and limits

- The host system toolchain remains unchanged. An isolated rustup installation under `/data/tmp`
  supplied pinned `nightly-2026-09-21`, Miri, rust-src, and cargo-fuzz 0.13.2 for the local evidence
  above. `make fuzz-asan-smoke` and `make miri-test` reproduce the repository commands on any host
  with that rustup toolchain and components installed.
- CI uses the same dated nightly and cargo-fuzz version. Its fuzz smoke retains the default
  AddressSanitizer leak detection and a 2 GiB per-process RSS limit; the local ptrace restriction is
  not encoded into repository defaults.
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
