#!/usr/bin/env python3
"""Run bounded release gates and preserve evidence; a pass is not production certification."""

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import uuid

from evidence_source import atomic_json, output_directory, sha256, source_snapshot, toolchain_snapshot
from package_evidence import verify as verify_package

MIN_AVAILABLE = 15 * 1024**3
START_AVAILABLE = MIN_AVAILABLE + 4 * 1024**3
GATES = {
    "check": ["make", "-j1", "check"],
    "go-race": ["make", "-j1", "go-race"],
    "msrv": ["make", "-j1", "msrv-check", "RNET_MSRV=1.85.0"],
    "fuzz-build": ["cargo", "check", "--locked", "--manifest-path", "fuzz/Cargo.toml", "--bins"],
    "fuzz-asan": ["make", "-j1", "fuzz-asan-smoke"],
    "miri": ["make", "-j1", "miri-test"],
    "supply-chain": ["make", "-j1", "dependency-check"],
    "package": ["make", "-j1", "package"],
    "package-smoke": ["make", "-j1", "package-smoke"],
}


def now():
    return datetime.now(timezone.utc).isoformat()


def memory_available():
    for line in Path("/proc/meminfo").read_text().splitlines():
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) * 1024
    raise OSError("MemAvailable is unavailable; cannot enforce the memory reserve")


def interrupt(*_):
    raise KeyboardInterrupt


def stop_process(process):
    """Only signal the new session owned by this gate, including its descendants."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait()


def skipped_resource_test(log):
    # These are the repository probe's explicit skip diagnostics, not generic "skip" words.
    markers = (b"SKIPPED game soak smoke:", b"SKIPPED remaining game soak smoke:")
    overlap = max(map(len, markers)) - 1
    tail = b""
    with log.open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            text = tail + block
            if any(marker in text for marker in markers):
                return True
            tail = text[-overlap:]
    return False


def run_command(root, output, name, command, *, require_clean=False,
                memory_reader=memory_available, timeout=3600):
    """Execute one gate. Tests inject only the host memory reader and child command.

    Logs are written directly to disk, so output size does not grow Python memory.
    A killed supervisor leaves a 'running' record, which the aggregator rejects.
    """
    if not math.isfinite(timeout) or timeout <= 0 or not command:
        raise ValueError("a positive timeout and nonempty command are required")
    root = Path(root).resolve()
    output = output_directory(root, output)
    log = output / "command.log"
    report = {"schema": 1, "gate": name, "command": list(command), "started_utc": now(),
              "status": "running", "reason": "", "exit_code": None,
              "source_before": None, "source_after": None,
              "run_id": os.environ.get("RNET_EVIDENCE_RUN_ID", str(uuid.uuid4())),
              "log_sha256": None}
    record = output / "report.json"
    atomic_json(record, report)
    process = None
    started = time.monotonic()
    try:
        report["source_before"] = source_snapshot(root)
        atomic_json(output / "environment.json", toolchain_snapshot())
        report["environment_sha256"] = sha256(output / "environment.json")
        atomic_json(record, report)
        with log.open("xb") as stream:
            if require_clean and report["source_before"]["dirty"]:
                report.update(status="blocked", reason="release evidence requires a clean source tree")
            elif memory_reader() < START_AVAILABLE:
                report.update(status="blocked", reason="memory below 19 GiB startup floor (15 GiB reserve plus build headroom)")
            else:
                env = dict(os.environ, CARGO_BUILD_JOBS="1", RUST_TEST_THREADS="1", RUST_TEST_NOCAPTURE="1")
                process = subprocess.Popen(command, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                           stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
                while process.poll() is None:
                    if memory_reader() < MIN_AVAILABLE:
                        report.update(status="blocked", reason="memory reserve below 15 GiB during execution")
                        break
                    if time.monotonic() - started >= timeout:
                        report.update(status="interrupted", reason="gate timeout")
                        break
                    time.sleep(0.1)
                if report["status"] == "running":
                    report.update(status="passed" if process.returncode == 0 else "failed",
                                  exit_code=process.returncode)
    except KeyboardInterrupt:
        report.update(status="interrupted", reason="supervisor interrupted")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        report.update(status="blocked", reason=str(error))
    finally:
        if process is not None:
            stop_process(process)
            report["exit_code"] = process.returncode
        try:
            report["source_after"] = source_snapshot(root)
            if report["status"] == "passed" and report["source_before"] != report["source_after"]:
                report.update(status="invalidated", reason="source inputs changed while the gate ran")
            if log.exists():
                report["log_sha256"] = sha256(log)
                if report["status"] == "passed" and skipped_resource_test(log):
                    report.update(status="blocked", reason="resource-limited test was skipped")
            if report["status"] == "passed" and name in ("package", "package-smoke"):
                package = root / "dist" / f"linux-{os.uname().machine}"
                report["package"] = verify_package(package, report["source_before"])
                report["package"]["archive_sha256"] = sha256(package.with_name(package.name + ".tar.gz"))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            report.update(status="invalidated", reason=f"cannot seal evidence: {error}")
        report.update(ended_utc=now(), elapsed_seconds=round(time.monotonic() - started, 3))
        atomic_json(record, report)
    print(f"{name}: {report['status']}; evidence={output}", flush=True)
    return 0 if report["status"] == "passed" else 1


def summarize(paths, required):
    """Fail closed on missing, duplicate, changed or mixed-source evidence."""
    problems, records, identities = [], {}, []
    for path in map(Path, paths):
        try:
            record = json.loads(path.read_text())
            name = record["gate"]
            if name in records:
                problems.append(f"duplicate gate: {name}")
            records[name] = record
            before = record["source_before"]
            if record["schema"] != 1 or record["status"] != "passed" or record["exit_code"] != 0:
                problems.append(f"gate not passed: {name}")
            if not before or before != record["source_after"] or before["dirty"]:
                problems.append(f"unclean or changed source: {name}")
            if sha256(path.parent / "command.log") != record["log_sha256"]:
                problems.append(f"log hash mismatch: {name}")
            if sha256(path.parent / "environment.json") != record["environment_sha256"]:
                problems.append(f"environment hash mismatch: {name}")
            if name in ("package", "package-smoke"):
                if record["package"]["source"] != before:
                    problems.append(f"package source mismatch: {name}")
            identities.append((before, record["run_id"]))
        except (OSError, ValueError, KeyError, TypeError) as error:
            problems.append(f"invalid evidence {path}: {error}")
    if not required or len(set(required)) != len(required):
        problems.append("required gates must be a nonempty unique list")
    for name in required:
        if name not in records:
            problems.append(f"missing gate: {name}")
    if identities and any(value != identities[0] for value in identities[1:]):
        problems.append("mixed source revisions or run IDs")
    if "package" in records and "package-smoke" in records:
        if records["package"].get("package") != records["package-smoke"].get("package"):
            problems.append("package build and smoke tested different artifacts")
    return {"schema": 1, "status": "passed" if not problems else "incomplete",
            "scope": "listed repository gates only; not target-environment certification",
            "required": list(required), "problems": problems,
            "source": identities[0][0] if identities else None,
            "reports": [str(path) for path in paths]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    sub = parser.add_subparsers(dest="operation", required=True)
    run = sub.add_parser("run")
    run.add_argument("--gate", choices=GATES, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--release", action="store_true")
    run.add_argument("--timeout", type=float, default=3600)
    summary = sub.add_parser("summarize")
    summary.add_argument("--require", nargs="+", choices=GATES, required=True)
    summary.add_argument("--reports", nargs="+", type=Path, required=True)
    summary.add_argument("--output", type=Path, required=True)
    suite = sub.add_parser("suite")
    suite.add_argument("--output", type=Path, required=True)
    suite.add_argument("--release", action="store_true")
    suite.add_argument("--timeout", type=float, default=3600)
    args = parser.parse_args()
    try:
        if args.operation == "run":
            # A fixed gate inventory prevents accidental secret-bearing commands in reports.
            result = run_command(args.root, args.output, args.gate, GATES[args.gate],
                                 require_clean=args.release, timeout=args.timeout)
            return result
        if args.operation == "suite":
            output = output_directory(args.root, args.output)
            os.environ.setdefault("RNET_EVIDENCE_RUN_ID", str(uuid.uuid4()))
            paths = []
            for name, command in GATES.items():
                path = output / name
                run_command(args.root, path, name, command, require_clean=args.release, timeout=args.timeout)
                paths.append(path / "report.json")
                if json.loads(paths[-1].read_text())["reason"] == "supervisor interrupted":
                    break
            report = summarize(paths, list(GATES))
            atomic_json(output / "summary.json", report)
            return 0 if report["status"] == "passed" else 1
        output = output_directory(args.root, args.output)
        report = summarize(args.reports, args.require)
        atomic_json(output / "summary.json", report)
        print(json.dumps(report, indent=2))
        return 0 if report["status"] == "passed" else 1
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"release evidence: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupt)
    raise SystemExit(main())
