# Game production qualification record

This record separates repository qualification from target-environment certification. It was
updated on 2026-09-22 against the dirty development tree shown by the commands below; rerun every
gate on the release commit and archive the resulting package hash before deployment.

## Repository gates

- Rust formatting, strict workspace Clippy, all workspace targets, structure limits, release ABI
  exports, C++11 final-link smoke tests, Go/cgo tests, and Go race tests pass.
- The Linux x86_64 SDK package was regenerated with its SHA256 manifest. Both shipped C++11 game
  examples compile against the packaged headers/static library and complete their loopback smoke
  flows; the package also contains the game guide and this qualification record.
- All six fuzz targets (`control_parser`, `datagram_preflight`, `ffi_config`, `frame_parser`,
  `game_wire`, and `kcp_engine`) completed five-second, one-job, 1 GiB RSS-limited smoke campaigns
  with release arithmetic and no sanitizer. A newly found impossible future KCP ACK timestamp was
  preserved as a regression and is rejected before entering the pinned dependency.
- `cargo audit` scanned 83 locked dependencies without a reported advisory. `cargo deny check`
  passed advisories, licenses, bans, and sources; the known duplicate `getrandom`, `syn`, and
  `windows-sys` versions remain visible warnings.
- A local three-second probe per transport used eight loopback clients, 256-byte payloads, and 100
  sends per client per second. TCP, UDP, and KCP each sent and received 2,248 messages with zero
  duplicates, reorder/too-old observations, backpressure, event drops, or protocol errors. The
  observed process RSS was 3.6–4.2 MiB and CPU was 10.4–12.3% on the recorded 48-core VM. This is a
  smoke result, not a capacity claim.

## Tooling limits

- ASan fuzzing cannot run with the installed stable-only Rust 1.96 toolchain: cargo-fuzz requires
  nightly `-Zsanitizer`, and this host has no `rustup` toolchain manager. Non-sanitized libFuzzer
  coverage and saved-crash replay pass.
- Miri is not installed and likewise requires a compatible nightly toolchain.
- The repository has deterministic concurrency/race regressions and Go race coverage, but no loom
  model. Adding loom would be a separate concurrency-modeling project rather than a release-command
  toggle.
- `snow` and the complete Noise/KCP integration have not received an independent third-party
  protocol audit.

## Final external gate

Do not label a target deployment certified until the release build completes the documented
24–72 hour TCP/UDP/KCP soak and `tc netem` loss, latency, jitter, duplication, and reorder matrix on
the actual kernel, hardware, network path, and application poll loop. Archive CPU, RSS, queue
gauges, close reasons, P95/P99/P99.9 latency, packet-quality counters, configuration, toolchain,
and package hash. Cross-instance recovery remains intentionally out of scope.
