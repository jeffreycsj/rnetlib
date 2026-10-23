#!/usr/bin/env python3
"""Bind a package to its build-start source, then verify every listed artifact."""

import argparse
import json
from pathlib import Path, PurePosixPath
import re

from evidence_source import atomic_json, sha256, source_snapshot, toolchain_snapshot


def seal(root, package, before):
    source = source_snapshot(root)
    if source != before["source"]:
        raise ValueError("source changed during package build; package is not sealed")
    atomic_json(Path(package) / "BUILD-INFO.json", {
        "schema": 1, "source": source, "environment": before["environment"],
        "qualification": "candidate only; build provenance is not test qualification"})


def verify(package, expected_source=None):
    package = Path(package)
    manifest = package / "SHA256SUMS"
    seen = set()
    for line in manifest.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if not match:
            raise ValueError("invalid package checksum entry")
        digest, name = match.groups()
        relative = PurePosixPath(name)
        if relative.is_absolute() or ".." in relative.parts or not relative.parts:
            raise ValueError("package checksum path escapes package")
        canonical = str(relative)
        if canonical in seen or canonical == "SHA256SUMS":
            raise ValueError("duplicate or self-referential package checksum")
        seen.add(canonical)
        path = package
        for part in relative.parts:
            path = path / part
            if path.is_symlink():
                raise ValueError("package checksum must not follow symlinks")
        if sha256(path) != digest:
            raise ValueError(f"package checksum mismatch: {name}")
    if "BUILD-INFO.json" not in seen:
        raise ValueError("package manifest does not cover build provenance")
    actual = set()
    for path in package.rglob("*"):
        if path.is_symlink():
            raise ValueError("package contains a symlink")
        if path.is_file() and path != manifest:
            actual.add(path.relative_to(package).as_posix())
    if actual != seen:
        raise ValueError("package inventory differs from checksum manifest")
    info = json.loads((package / "BUILD-INFO.json").read_text())
    if info.get("schema") != 1 or not isinstance(info.get("source"), dict):
        raise ValueError("unsupported package provenance")
    if expected_source is not None and info["source"] != expected_source:
        raise ValueError("package source does not match gate source")
    return {"manifest_sha256": sha256(manifest), "source": info["source"], "files": len(seen)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    sub = parser.add_subparsers(dest="operation", required=True)
    capture = sub.add_parser("capture")
    capture.add_argument("--output", type=Path, required=True)
    end = sub.add_parser("seal")
    end.add_argument("--before", type=Path, required=True)
    end.add_argument("--package", type=Path, required=True)
    check = sub.add_parser("verify")
    check.add_argument("--package", type=Path, required=True)
    args = parser.parse_args()
    if args.operation == "capture":
        atomic_json(args.output, {"source": source_snapshot(args.root), "environment": toolchain_snapshot()})
    elif args.operation == "seal":
        seal(args.root, args.package, json.loads(args.before.read_text()))
    else:
        print(json.dumps(verify(args.package), indent=2))


if __name__ == "__main__":
    main()
