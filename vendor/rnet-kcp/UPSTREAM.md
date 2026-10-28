# KCP fork provenance

Source: <https://github.com/Matrix-Zhang/kcp>, crates.io `kcp` **0.6.0**.
Source archive SHA-256:
`9be8f8058681ab8b7e4959a1ee88a4446f43b7c787f174d9d1b32ba493737a59`.
`LICENSE` preserves the upstream MIT notice. The original `src/lib.rs`,
`src/error.rs` and `src/kcp.rs` were imported from that archive.

This is an internal direct dependency, not a Cargo root patch. Downstream Rust
consumers must not disable overflow checks. It is not a new network protocol or
an independently audited implementation. Keep the fork isolated so changes can
be compared to the pinned source; the upstream optional features are retained.

Patch scope:

- Clock and sequence arithmetic uses explicit modulo-2^32 operations and
  half-space ordering, including selective ACK lookup across sequence rollover.
- Durations, retry counters, congestion budgets and buffer sizes are not serial
  numbers: bounds/checks prevent them from silently wrapping.
- Retransmission backoff is capped at the existing 60-second RTO maximum; retry
  counts saturate instead of returning to zero (which means "never transmitted").
  MTU is restricted to 50..=65535 before any state change/allocation. This does
  not tighten RNet's already smaller transport MTU range.
- RTT weighted averages use wider intermediates. Fragment counts widen before
  addition; fragment ceiling division avoids adding to the payload length.
  Other size arithmetic relies on the bounded MTU/window or allocated byte lengths.
- Input copies validated data directly, avoiding an uninitialized `BytesMut` slice.
  Clippy/rustfmt mechanical cleanup is included; no lint suppression was added.
- Boundary tests inject sequence state only inside this crate; consumer tests
  use the public adapter and genuine fragmented traffic without workspace profiles.

Maintenance: review upstream releases/security notices whenever dependencies are
updated. `upstream.Cargo.lock` retains the original registry identity for the
dependency gate's advisory scan; the main/fuzz graphs audit the fork's actual
dependencies. A clean scan of the renamed local crate alone is **not** sufficient.
On an upstream update, update this provenance, identity lock, patch review and tests
together. No vulnerability exception is granted by vendoring.
