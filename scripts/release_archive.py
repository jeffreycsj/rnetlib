#!/usr/bin/env python3
"""Prepare or verify offline Release attachments; never contact GitHub or publish."""
import argparse
import gzip
import json
from pathlib import Path
import sys
import tarfile
import tempfile

from archive_io import MAX_MEMBER, extract_tar, inventory, no_symlinks, read_json, regular_file
from archive_validation import EVIDENCE_FILES, SDK_NAME, checked_identity, verify_inputs
from evidence_source import sha256

SCOPE = "offline consistency only; not origin or approval authentication"
ATTACHMENTS = {SDK_NAME, SDK_NAME + ".sha256", "evidence.tar.gz", "evidence.tar.gz.sha256"}
MANIFEST = "archive-manifest.json"


def _copy(source, target):
    """Stream into a new file, retaining limits even if a source unexpectedly grows."""
    source = regular_file(source)
    target.parent.mkdir(parents=True, exist_ok=True)
    total = 0
    with source.open("rb") as incoming, target.open("xb") as outgoing:
        for block in iter(lambda: incoming.read(65536), b""):
            total += len(block)
            if total > MAX_MEMBER:
                raise ValueError("input grew beyond archive file limit")
            outgoing.write(block)


def _record(path):
    path = regular_file(path)
    return {"sha256": sha256(path), "size": path.stat().st_size}


def _pack_evidence(evidence, path):
    # Stable metadata makes retries byte-identical; source reports remain unmodified.
    with path.open("xb") as output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
                for name in sorted(EVIDENCE_FILES):
                    source = evidence / name
                    entry = tarfile.TarInfo("evidence/" + name)
                    entry.size, entry.mode = source.stat().st_size, 0o644
                    with source.open("rb") as stream:
                        archive.addfile(entry, stream)


def verify_archive(directory, identity):
    """Check attachments and recompute their evidence against independent identity.

    A manifest's hashes are not a signature. Only trusted hosting metadata and an
    approved publisher can authenticate the run; this function makes no such claim.
    """
    directory = Path(directory)
    identity = checked_identity(identity)
    if inventory(directory) != ATTACHMENTS | {MANIFEST}:
        raise ValueError("archive attachment inventory is incomplete or unexpected")
    manifest = read_json(directory / MANIFEST)
    if (set(manifest) != {"schema", "scope", "identity", "source", "package", "gates", "artifacts"}
            or type(manifest.get("schema")) is not int or manifest["schema"] != 1
            or manifest.get("scope") != SCOPE or manifest.get("identity") != identity
            or manifest.get("artifacts") != {name: _record(directory / name) for name in ATTACHMENTS}):
        raise ValueError("archive manifest identity or attachment digest mismatch")
    bundle = directory / "evidence.tar.gz"
    expected = f"{sha256(bundle)}  evidence.tar.gz\n"
    if regular_file(directory / "evidence.tar.gz.sha256", 1024).read_text() != expected:
        raise ValueError("evidence bundle checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="rnet-archive-verify-") as temporary:
        unpacked = Path(temporary) / "unpacked"
        extract_tar(bundle, unpacked)
        if {path.name for path in unpacked.iterdir()} != {"evidence"}:
            raise ValueError("evidence bundle root is invalid")
        verified = verify_inputs(unpacked / "evidence", directory / SDK_NAME, identity)
    if any(manifest.get(key) != value for key, value in verified.items()):
        raise ValueError("archive manifest differs from recomputed evidence")
    return manifest


def prepare_archive(evidence, sdk, identity, output):
    """Snapshot, validate and seal new attachments without overwriting any output.

    Verification failures expose no ready output. A filesystem failure during the
    final copy may leave an incomplete output; it is never considered valid without
    a manifest written last and a successful verify. Retry with a fresh output path.
    """
    evidence, sdk = Path(evidence), Path(sdk)
    identity = checked_identity(identity)
    output = no_symlinks(output)
    if output.exists():
        raise FileExistsError("archive output already exists; refusing to overwrite")
    if output.is_relative_to(evidence.absolute()):
        raise ValueError("archive output must not be inside input evidence")
    if sdk.name != SDK_NAME or inventory(evidence) != EVIDENCE_FILES:
        raise ValueError("unexpected SDK name or incomplete evidence inventory")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rnet-archive-stage-", dir=output.parent) as temporary:
        staging = Path(temporary)
        snapshot, ready = staging / "evidence", staging / "ready"
        ready.mkdir()
        for name in sorted(EVIDENCE_FILES):
            _copy(evidence / name, snapshot / name)
        _copy(sdk, ready / SDK_NAME)
        _copy(Path(str(sdk) + ".sha256"), ready / (SDK_NAME + ".sha256"))
        verified = verify_inputs(snapshot, ready / SDK_NAME, identity)
        _pack_evidence(snapshot, ready / "evidence.tar.gz")
        (ready / "evidence.tar.gz.sha256").write_text(
            f"{sha256(ready / 'evidence.tar.gz')}  evidence.tar.gz\n")
        manifest = {"schema": 1, "scope": SCOPE, **verified,
                    "artifacts": {name: _record(ready / name) for name in sorted(ATTACHMENTS)}}
        (ready / MANIFEST).write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        verify_archive(ready, identity)
        output.mkdir(exist_ok=False)
        for name in [*sorted(ATTACHMENTS), MANIFEST]:
            _copy(ready / name, output / name)
    return verify_archive(output, identity)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="operation", required=True)
    build = sub.add_parser("prepare", help="validate inputs and create new attachments")
    build.add_argument("--identity", type=Path, required=True)
    build.add_argument("--evidence", type=Path, required=True)
    build.add_argument("--sdk", type=Path, required=True)
    build.add_argument("--output", type=Path, required=True)
    check = sub.add_parser("verify", help="recompute the archived evidence, without original paths")
    check.add_argument("--identity", type=Path, required=True)
    check.add_argument("--archive-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        identity = read_json(args.identity)
        if args.operation == "prepare":
            result = prepare_archive(args.evidence, args.sdk, identity, args.output)
        else:
            result = verify_archive(args.archive_dir, identity)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"release archive: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
