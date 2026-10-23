# Generated C interface

Rust FFI declarations and documentation are the source of `include/rnet.h`.
Do not edit the generated file. The fixed cbindgen build dependency is locked in
Cargo.lock; producers do not install a separate global cbindgen executable.

```sh
make header-update     # Explicitly refresh the tracked SDK header, then review it.
make header-check      # Regenerate and compare; never rewrites the SDK copy.
make abi-check         # Also verify the approved exported symbol list.
```

Cargo builds of `rnet-ffi` always provide the header in that build's `OUT_DIR`.
They do not modify the checkout, including when invoked by IDEs or from read-only
sources. `scripts/generated_header.py` locates the matching output using Cargo's
package ID and structured build messages, not an assumed hashed directory. It
accepts `--release`, `--target-dir PATH`, and `--output PATH`; use `--check` or
`--update` explicitly. The CARGO_TARGET_DIR environment variable is also honored.
Relative paths are relative to the repository when invoked from its root.

Generation/metadata failures abort before copying. Explicit updates replace a
complete file atomically; unchanged output retains its mtime. The checker builds
with `--locked` and reports drift without fixing it, so CI cannot silently hide a
forgotten SDK update. Normal build dependencies may need downloading initially;
offline builds require those dependencies already cached.

## Maintaining declarations and contracts

- Change declarations and comments in the relevant `crates/rnet-ffi/src` module.
  `cbindgen.toml` defines public spelling, include guards and common C usage notes;
  it does not contain hand-written function prototypes or record layouts.
- Rust layout types retain their original names. `abi_names.rs` supplies public C
  typedef aliases; both `struct rnet_*` tags and `rnet_*_t` aliases remain usable.
- FFI constants use explicit ABI values. Constants previously derived from native
  enums have compile-time equality checks, so changing the native value cannot
  silently change the C contract. Generated constants are macros, not anonymous
  enum members; they still work as integer constant expressions in C/C++ and cgo.
  They are now visible to `#ifdef`, and C++ `decltype`/template deduction sees integer
  literals rather than anonymous enum types. Those representation details are not
  preserved; ordinary SDK call sites remain unchanged. Do not use them to infer
  compatibility. Only `rnet_abi_version` is the runtime ABI check.
- Nullable callback fields use inline function pointer types because this pinned
  generator cannot resolve `Option<callback typedef>`. Compile-time assignments
  verify their equality with the public bare Rust callback aliases; the C calling
  convention, nullability and layouts do not change.
- Keep function, field, alias and constant documentation on their Rust declaration.
  The native documentation gate also covers generated constant macros. Neither
  presence checks nor generation proves the behavioral description is correct.
- Review signatures, values and layout, not just textual changes. Run the FFI,
  C/C++11, Go and C# regressions for ABI changes. Formatting-only changes do not
  justify changing the SDK ABI discriminator or adding legacy SDK paths.

## Packaging

`make package` checks the generated header against the SDK copy using a release
build before replacing the package directory. It ships that verified copy beside
the static/dynamic libraries. Consumers use these artifacts directly; Python,
cbindgen and the Rust compiler are producer tools, not C/C++/C# runtime dependencies.
The cbindgen MPL-2.0 exception in the dependency policy is scoped to that build-time
generator; it is not linked into the network library.

This automation prevents a manually stale header, not unsupported platform ABI
differences or peer protocol incompatibility. Cross-platform qualification and
production soak remain separate release work.
