# Phase 2 dependency review

## Noise and cryptography

- `snow = 0.10.0`, MIT/Apache-2.0, implements Noise revision 34. The version is newer than the 0.9.5 stateful transport nonce-desynchronization security fix and the 0.9.6 PSK validation fix.
- `ring = 0.17.14`, ISC-style license, is used for HMAC cookie generation and OS randomness.
- `zeroize = 1.9.0`, MIT/Apache-2.0, clears Rust-owned static private-key buffers on drop.
- The selected handshake is `Noise_XX_25519_ChaChaPoly_BLAKE2s`. Server identity is meaningful only because the client pins the expected server public key; XX without out-of-band verification would not prevent an active attacker.
- `snow` is not claimed to have a completed formal audit. This is an explicit production qualification risk, not a hidden assumption.

## KCP

- `kcp = 0.6.0`, MIT, repository `Matrix-Zhang/kcp`, describes itself as a Rust translation of the reference KCP implementation.
- The crate is isolated behind `KcpEngine`; RNet controls MTU 1200, 128/128 windows, 10 ms interval, nodelay, resend 2, and congestion-control disablement.
- The adapter has deterministic loss/reordering recovery coverage. End-to-end KCP additionally passes Noise handshake, explicit application authorization, 4096-byte message transfer, and shutdown tests.
- Each connection negotiates a random nonzero conversation in the authenticated-cookie preflight. RNet rejects stale conversations and impossible future/older-than-60-second ACK timestamps before they enter the dependency; saved fuzz input and signed-wrap boundary regressions cover both cases.
- KCP alone is built with wrapping arithmetic because its sequence calculations require modulo-2^32 behavior; overflow checks remain enabled for RNet workspace code.
- The fuzz workspace explicitly enables release-profile debug assertions and overflow checks,
  with only KCP exempted from overflow checks. This matches cargo-fuzz's compiler flags so
  Cargo emits the KCP override; relying on implicit release defaults can accidentally enable
  checked subtraction in KCP's serial comparisons. The 2026-10-26 ASan gate exposed this
  configuration mismatch, and replaying its saved input after the profile fix succeeded.
- Residual qualification gaps are long-duration sanitizer/fuzz campaigns, broad `tc netem`
  matrices, independent dependency review, and production tuning against target latency/bandwidth
  profiles. Short ASan campaigns exercise all six fuzz boundaries, while Miri covers the pure core
  and protocol crates rather than the complete Noise/KCP stack.

## Toolchain

- The workspace declares Rust 1.85 as its MSRV, and CI pins 1.85.0 for a locked, all-target compilation gate.
- The final local production gate recorded in the qualification reports used TencentOS Rust 1.96.0. Formatting and Clippy do not replace the system compiler or PATH.
- The historical dependency gate scanned 104 locked packages; that result does not qualify later lockfiles. Current checks and their source fingerprints must be recorded separately.

## Tokio safety update (2026-10-26)

The fresh dependency gate rejected Tokio 1.53.1 under
[RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html).
The advisory concerns I/O ownership in the Unix `AsyncFd` constructors and a possible
use-after-free. No direct use of these constructors was found in RNet; this is not evidence
of an exploitable RNet packet path, nor a reason to waive a dependency safety advisory.

The workspace minimum and both the application and fuzz lockfiles now use Tokio 1.53.3,
a patched version. Raising the manifest minimum also prevents downstream Cargo consumers
from selecting the older affected versions when they do not use this repository's lockfile.
No advisory ignore entry or security-policy relaxation was added. Keep the original failed
gate as evidence and rerun compilation, transport/SDK tests and specialist gates against
the updated source. This dependency change is not target-environment certification.

The exact repository results, sanitizer/Miri scope, short transport probe, and final external gates
are recorded in [game-production-qualification.md](game-production-qualification.md).
