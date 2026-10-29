"""Synthetic records only; these fixtures are never release-qualification evidence."""
import json
from pathlib import Path
import sys
import tarfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from evidence_source import sha256
from package_evidence import verify
from release_evidence import GATES, summarize


def write_json(path, value):
    path.write_text(json.dumps(value))


def fixture(root):
    identity = {"repository": "example/netlib", "commit": "a" * 40,
                "tag": "candidate-001", "tag_commit": "a" * 40,
                "run_id": "101", "attempt": "2", "validator_revision": "b" * 40}
    source = {"revision": identity["commit"], "dirty": False,
              "input_sha256": "c" * 64, "cargo_lock_sha256": "d" * 64}
    package = root / "linux-x86_64"
    package.mkdir(parents=True)
    write_json(package / "BUILD-INFO.json", {"schema": 1, "source": source})
    (package / "librnet.a").write_bytes(b"synthetic test SDK; not executable")
    (package / "SHA256SUMS").write_text("".join(
        f"{sha256(package / name)}  ./{name}\n" for name in ("BUILD-INFO.json", "librnet.a")))
    archive = root / "linux-x86_64.tar.gz"
    with tarfile.open(archive, "w:gz", format=tarfile.GNU_FORMAT) as stream:
        stream.add(package, arcname=package.name)
    Path(str(archive) + ".sha256").write_text(f"{sha256(archive)}  {archive.name}\n")
    package_record = {**verify(package), "archive_sha256": sha256(archive)}
    evidence = root / "evidence"
    evidence.mkdir()
    reports = []
    for gate, command in GATES.items():
        directory = evidence / gate
        directory.mkdir()
        (directory / "command.log").write_text(f"SYNTHETIC fixture for {gate}\n")
        write_json(directory / "environment.json", {"fixture": True})
        report = {"schema": 1, "gate": gate, "command": command, "status": "passed",
                  "reason": "", "exit_code": 0, "run_id": "101-2",
                  "source_before": source, "source_after": source,
                  "log_sha256": sha256(directory / "command.log"),
                  "environment_sha256": sha256(directory / "environment.json")}
        if gate in ("package", "package-smoke"):
            report["package"] = package_record
        path = directory / "report.json"
        write_json(path, report)
        reports.append(path)
    write_json(evidence / "summary.json", summarize(reports, list(GATES)))
    return evidence, archive, identity
