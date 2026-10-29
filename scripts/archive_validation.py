"""Offline consistency checks, not authentication of a GitHub run or approval.

The future publisher must obtain expected identity from trusted GitHub metadata.
Self-consistent hashes alone do not prove origin, test execution or production fitness.
"""
from pathlib import Path, PurePosixPath
import re
import tempfile

from archive_io import MAX_BYTES, extract_tar, inventory, read_json, regular_file
from evidence_source import sha256
from package_evidence import verify as verify_package
from release_evidence import GATES, skipped_resource_test, summarize

SDK_NAME = "linux-x86_64.tar.gz"
EVIDENCE_FILES = {"summary.json"} | {
    f"{gate}/{name}" for gate in GATES
    for name in ("report.json", "command.log", "environment.json")}


def checked_identity(identity):
    """Use a conservative tag subset; do not create or resolve remote tags here."""
    patterns = {"repository": r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+",
                "commit": r"[0-9a-f]{40}", "tag_commit": r"[0-9a-f]{40}",
                "validator_revision": r"[0-9a-f]{40}",
                "run_id": r"[1-9][0-9]{0,19}", "attempt": r"[1-9][0-9]{0,9}",
                "tag": r"[A-Za-z0-9][A-Za-z0-9._/-]{0,127}"}
    if not isinstance(identity, dict) or identity.keys() != patterns.keys():
        raise ValueError("invalid archive identity fields")
    for key, pattern in patterns.items():
        if not isinstance(identity[key], str) or not re.fullmatch(pattern, identity[key]):
            raise ValueError(f"invalid archive identity: {key}")
    if (identity["commit"] != identity["tag_commit"] or ".." in identity["tag"]
            or any(not part or part.startswith(".") or part.endswith((".", ".lock"))
                   for part in identity["tag"].split("/"))):
        raise ValueError("invalid tag or tag does not resolve to the qualified commit")
    return dict(identity)


def _source(value, commit):
    if (not isinstance(value, dict)
            or set(value) != {"revision", "dirty", "input_sha256", "cargo_lock_sha256"}
            or value["revision"] != commit or value["dirty"] is not False
            or any(not isinstance(value[key], str) or not re.fullmatch(r"[0-9a-f]{64}", value[key])
                   for key in ("input_sha256", "cargo_lock_sha256"))):
        raise ValueError("invalid, dirty or unexpected source identity")
    return value


def _schema_one(record):
    if type(record.get("schema")) is not int or record["schema"] != 1:
        raise ValueError("unsupported evidence schema")


def verify_inputs(evidence, archive, identity):
    """Recompute every gate and SDK binding using fixed relative paths.

    Historical summary paths are metadata only and are never opened. Inputs must
    be quiescent, caller-owned copies. Returned identity remains an offline claim.
    """
    identity = checked_identity(identity)
    evidence, archive = Path(evidence), Path(archive)
    if inventory(evidence) != EVIDENCE_FILES:
        raise ValueError("missing or unexpected evidence files")
    original = read_json(evidence / "summary.json")
    _schema_one(original)
    source = _source(original.get("source"), identity["commit"])
    paths, records = [], {}
    for gate, command in GATES.items():
        path = evidence / gate / "report.json"
        record = read_json(path)
        _schema_one(record)
        if (record.get("gate") != gate or record.get("command") != command
                or record.get("status") != "passed" or record.get("reason") != ""
                or type(record.get("exit_code")) is not int or record["exit_code"] != 0
                or record.get("run_id") != f"{identity['run_id']}-{identity['attempt']}"
                or record.get("source_before") != source or record.get("source_after") != source):
            raise ValueError(f"invalid or mismatched gate record: {gate}")
        # Validate booleans explicitly: JSON 0 must not masquerade as clean=false.
        _source(record["source_before"], identity["commit"])
        _source(record["source_after"], identity["commit"])
        read_json(path.parent / "environment.json")
        if skipped_resource_test(path.parent / "command.log"):
            raise ValueError(f"resource-limited test was skipped: {gate}")
        records[gate] = record
        paths.append(path)
    recomputed = summarize(paths, list(GATES))
    if recomputed["status"] != "passed":
        raise ValueError("invalid evidence: " + "; ".join(recomputed["problems"]))
    if any(original.get(key) != recomputed[key]
           for key in ("schema", "status", "required", "problems", "source", "scope")):
        raise ValueError("summary does not describe the complete verified gate set")
    old_paths = original.get("reports")
    if (not isinstance(old_paths, list) or len(old_paths) != len(GATES)
            or any(not isinstance(path, str) or PurePosixPath(path).name != "report.json"
                   for path in old_paths)
            or {PurePosixPath(path).parent.name for path in old_paths} != set(GATES)):
        raise ValueError("summary contains missing or duplicate gate references")

    if archive.name != SDK_NAME:
        raise ValueError("only the qualified Linux x86_64 SDK archive is supported")
    regular_file(archive, MAX_BYTES)
    checksum = regular_file(Path(str(archive) + ".sha256"), 1024)
    digest = sha256(archive)
    if checksum.read_text() != f"{digest}  {SDK_NAME}\n":
        raise ValueError("SDK archive checksum mismatch")
    package_record = records["package"].get("package")
    if not isinstance(package_record, dict) or package_record.get("archive_sha256") != digest:
        raise ValueError("SDK archive differs from the qualified artifact")
    with tempfile.TemporaryDirectory(prefix="rnet-archive-sdk-") as temporary:
        unpacked = Path(temporary) / "unpacked"
        extract_tar(archive, unpacked)
        if {path.name for path in unpacked.iterdir()} != {"linux-x86_64"}:
            raise ValueError("SDK archive root is invalid")
        package = unpacked / "linux-x86_64"
        inventory(package)
        read_json(package / "BUILD-INFO.json")
        regular_file(package / "SHA256SUMS", 1024**2)
        verified = {**verify_package(package, source), "archive_sha256": digest}
    if package_record != verified:
        raise ValueError("SDK manifest or inventory differs from qualification")
    return {"identity": identity, "source": source, "package": verified, "gates": list(GATES)}
