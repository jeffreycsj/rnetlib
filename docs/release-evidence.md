# Release evidence (Linux)

Evidence describes a specific source and set of gates. It does not certify target
capacity, weak-network behavior, other platforms, or independent security review.

## Run gates

Use a dedicated checkout and a resource-qualified Linux x86_64 machine. Keep at
least 19 GiB **available** at startup (15 GiB reserve plus 4 GiB headroom); the supervisor
refuses low-memory starts and stops its child process group on pressure. Do not
run other heavy tasks concurrently. Existing ordinary CI remains a development
gate, not a substitute for this qualification run.

```sh
# Unique output directories are required; existing evidence is never overwritten.
python3 -B scripts/release_evidence.py suite --release \
  --output target/release-evidence/candidate-001

# Development checks can run on a dirty tree, clearly labeled as such.
python3 -B scripts/release_evidence.py run --gate check \
  --output target/release-evidence/dev-check-001
```

The suite serially runs `check` (including Loom), Go race with rebuilt native
archives, Rust 1.85, fuzz compilation, ASan fuzz smoke, Miri, dependency checks,
packaging and actual packaged C/C++11/Go/C# TCP/UDP/KCP examples. It uses existing
Make targets rather than defining a second test suite. Provision the documented
nightly, cargo-fuzz, cargo-audit/cargo-deny, native compilers and .NET before use;
missing tools are failed/blocked gates, never silently skipped. `DOTNET` can select
an SDK outside PATH. `--timeout` is a positive per-gate deadline (default 3600 s).
The suite still consists of short repository checks, not a long fuzz campaign.

`dependency-check` audits both the root and independent fuzz lockfiles. Advisory warnings
are failures, including informational safety advisories; `cargo deny` applies the same
repository policy to both dependency graphs with `--locked`, so qualification cannot
silently re-resolve them. Policy-permitted duplicate-version warnings remain visible.
Any failed check stops this gate. This covers the fuzz harness dependencies as well as
the shipped library; it does not constitute an independent dependency security audit.
The approved NCSA exception is restricted to fuzz-only `libfuzzer-sys = 0.4.13`;
upgrading it requires renewed license review. NCSA is not globally allowed, and
the exception does not relax advisory checks.

Provision a trusted `promtool` too (`PROMTOOL` selects its path). `make check` now runs
offline operations-rule checks, including counter reset, absent samples, target failure
and per-runtime matching. Packaging ships these rules; package-smoke validates the actual
archived copies. Toolchain evidence records the validator version. No monitoring server
is started or production configuration applied by these gates.

`report.json` records the actual command, UTC timestamps, exit code, source
revision/dirty state/input digest, lockfile hash, run ID and hashes of the log and
toolchain record. `command.log` is written directly to disk without retaining it
in supervisor memory; tail it while the gate runs. Ensure adequate disk space.
No complete environment dump is collected; test output itself must not contain
secrets. Source hashes cover tracked and nonignored untracked files, including
deletions/modes/symlinks, but exclude ignored build/cache artifacts.

Status meanings:

- `passed`: this command exited zero with stable inputs and intact evidence.
- `failed`: command exited nonzero.
- `blocked`: missing prerequisite, memory reserve failure, or recognized
  resource-skipped soak/probe test. Test stdout/stderr are uncaptured in evidence runs
  so successful-but-skipped tests cannot hide their resource notice.
- `interrupted`: supervisor interruption or timeout.
- `invalidated`: source changed or final evidence/package verification failed.
- `running`: incomplete record, including abrupt supervisor termination; not a pass.

`summary.json` rejects missing/duplicate gates, changed logs/environments, dirty
inputs and mixed source/run IDs. The CLI only exposes fixed gate commands. For
separate invocations to be aggregated, explicitly share `RNET_EVIDENCE_RUN_ID`
and use `summarize --require ... --reports ... --output ...`; do not mix historical
reports. A subset summary only covers that subset, not complete qualification.
Hashes detect accidental mismatches; they are not signatures or tamper-proof audit.

## Package provenance

`make package` captures source and toolchain identity before building, refuses to
seal if source changed, writes `BUILD-INFO.json`, then covers it in `SHA256SUMS`.
Ordinary dirty-tree packages remain permitted and explicitly labeled candidates.
The adjacent `.tar.gz` and its checksum preserve executable modes for CI downloads.
The `package` and `package-smoke` evidence records reference the same package
manifest hash and source identity. Package reports live outside the package,
avoiding a self-referential checksum. Do not rebuild between the two gates.

`make package-smoke` verifies and extracts the actual local archive, checks executable
modes and listed files, and compiles examples against the shipped
native archive and managed DLL, and runs all three transports. Its scratch builds
also run the independent probe tests against the archived `game_probe` executable.
Test credentials are temporary and are never packaged. All scratch builds
remain under target; it does not use the source-tree Go/native library. Both gates
must pass before the package has repository-level qualification evidence. Verification
also rejects unlisted files and symlinks; keep example build output outside the package.

## CI and remaining assurance

Ordinary CI's supply-chain job invokes the same `make -j1 dependency-check` gate,
using cargo-audit 0.22.2 and cargo-deny 0.20.2 with locked installation dependencies.
It checks both graphs and propagates failures, rather than maintaining separate
action defaults that may omit the fuzz workspace. It does not replace the full suite.

The ordinary Linux SDK job requires 19 GiB available before building and sets
`RNET_REQUIRE_RESOURCE_TESTS=1`. Probe/soak resource refusals then fail the test,
including pressure detected after startup, rather than returning a passing skip.
This flag is inherited by packaged probe tests too. Insufficient runner memory
blocks SDK upload; provision an appropriate runner separately instead of lowering
the reserve. Local developers may omit the flag to retain explicit resource skips;
the qualification supervisor still rejects those skip markers.

The manual `release-qualification` workflow uses a **pre-provisioned**, dedicated
self-hosted runner labeled `rnet-qualification`, with Linux/X64 labels. It does not
create a machine, install system software, publish a release or run target soak.
Only dispatch trusted commits on this runner; do not execute untrusted PR code.
It archives evidence even after failure; packages are uploaded only on success.
Keep durable release records beyond the workflow artifact retention period.
The manual Release archive design (`specs/release-evidence-archive/spec.md` in the
source checkout) is being implemented. The offline preparation and verification
stage below is available; GitHub origin checks, approval and publishing are not
implemented or enabled yet. It is not an active long-term archive service.

Existing CI additionally runs `make package-smoke` after packaging and before
uploading its development SDK. This checks the actual archive and the shipped
C/C++11/Go/C# examples for TCP/UDP/KCP, not just the build-tree library.
It is not represented as a complete evidence suite. Artifact upload follows the
[official action interface](https://github.com/actions/upload-artifact/tree/v4).

Source snapshots at start/end cannot prove absence of transient changes reverted
mid-run, reused compiler caches, or a compromised host. Release infrastructure
must provide an isolated trusted checkout and controlled toolchains. A clean
repository-gate result still needs the documented independent review, real game
integration, target weak-network matrix and final 24–72 hour soak.

## Prepare and recheck offline Release attachments

Run these source-checkout tools on a resource-qualified machine as described above.
They never contact GitHub, create tags, request permissions, execute downloaded SDK
code or publish anything. Successful verification means **offline consistency only**,
not authenticated origin, human approval or production qualification.

Provide a separate operator-owned identity JSON file with these string fields:

| Field | Meaning |
| --- | --- |
| `repository` | Expected GitHub `owner/repository`. |
| `commit` | Full lowercase 40-hex qualified source SHA, not a branch name. |
| `tag` | Existing candidate tag; restricted to a conservative Git-ref character subset. |
| `tag_commit` | Full commit SHA obtained by resolving that tag; must equal `commit`. |
| `run_id` | Positive decimal GitHub qualification run ID. |
| `attempt` | Positive decimal attempt; the reports must contain `run_id-attempt`. |
| `validator_revision` | Full 40-hex trusted validator revision; never a script downloaded from evidence. |

Do not derive expected identity from the bundle under test. The CLI only checks
these supplied values for syntax and consistency; it cannot establish that a real
GitHub run, tag or reviewer matches them. The future publisher must obtain those
facts from trusted hosting metadata. Arbitrary local UUID run IDs are not accepted
as GitHub qualification runs.

With all nine gates passed on one clean source and one run/attempt, run:

```sh
# Replace these example paths with the exact qualified artifacts and identity file.
# The SDK checksum must be adjacent: linux-x86_64.tar.gz.sha256.
python3 -B scripts/release_archive.py prepare \
  --identity /operator/expected-identity.json \
  --evidence target/release-evidence/123456-1 \
  --sdk dist/linux-x86_64.tar.gz \
  --output target/release-archive/candidate-001

# Does not need the original evidence directory or SDK build tree.
python3 -B scripts/release_archive.py verify \
  --identity /operator/expected-identity.json \
  --archive-dir target/release-archive/candidate-001
```

The five output files are the unchanged SDK archive and checksum, deterministic
`evidence.tar.gz` and its checksum, and `archive-manifest.json` binding their sizes,
hashes, source and expected identity. Original reports, logs and environment records
are preserved byte-for-byte, including historical paths. Verification opens only
fixed relative paths and recomputes the gate and inner SDK manifest checks; a
summary claiming `passed` is insufficient. Missing gates, unexpected files, resource
skip markers and inconsistent build/smoke package records fail closed.

Inputs and scratch directories must be caller-owned and free of concurrent writers.
Preparation snapshots inputs, verifies them and writes the manifest last to a new
output directory. Existing outputs are never overwritten. Filesystem errors during
the final copy may leave incomplete output; verify rejects it, and retry must use a
fresh path. The CLI returns nonzero on failure, not a partial success report.

Current bounds: 10,000 entries, 256 MiB per regular file, 1 GiB total inventory or
expanded tar stream (including headers/padding), and 1 MiB per JSON object file.
Extraction rejects links, devices, sparse/PAX metadata, noncanonical paths and
duplicate entries; permissions/ownership from archives are not restored. These
limits are deliberate, not silently relaxed for large inputs. Keep adequate scratch
disk space and the 15 GiB memory reserve. Review logs for secrets before any future
publication. Hashes are unsigned and do not prevent an attacker forging an entirely
self-consistent set; origin validation and approval remain mandatory next steps.
