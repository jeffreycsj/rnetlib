#!/usr/bin/env python3
"""Verify GitHub qualification artifacts; publish only from the approved workflow job."""
import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import threading
import zipfile

from archive_io import MAX_BYTES, inventory, no_symlinks
from archive_validation import EVIDENCE_FILES, SDK_NAME, checked_identity
from archive_zip import extract_zip
from evidence_source import sha256
from github_archive_http import GitHub
from github_archive_origin import WORKFLOW, collect_origin, prefix
from github_archive_publish import file_binding, publish
from release_archive import prepare_archive
from release_evidence import MIN_AVAILABLE, START_AVAILABLE, memory_available


def prepare_hosted(api, identity, context, output):
    """Download data into new owned scratch, authenticate ZIP hashes, then validate contents."""
    origin = collect_origin(api, identity, context)
    output = no_symlinks(output)
    output.mkdir(parents=True, exist_ok=False)
    p = prefix(identity["repository"])
    with tempfile.TemporaryDirectory(prefix="download-", dir=output) as temporary:
        scratch = Path(temporary)
        for kind, record in origin["artifacts"].items():
            path = scratch / (kind + ".zip")
            api.download(p + f"/actions/artifacts/{record['id']}/zip", path, MAX_BYTES)
            if path.stat().st_size != record["size_in_bytes"] or "sha256:" + sha256(path) != record["digest"]:
                raise ValueError("download does not match GitHub artifact digest/size")
            extract_zip(path, scratch / kind)
        if inventory(scratch / "evidence") != EVIDENCE_FILES or inventory(scratch / "candidate") != {SDK_NAME, SDK_NAME + ".sha256"}:
            raise ValueError("unexpected qualification artifact layout")
        directory = output / "attachments"
        prepare_archive(scratch / "evidence", scratch / "candidate" / SDK_NAME, identity, directory)
    # Preserve hosted provenance outside the unchanged five-file offline format.
    receipt = output / "hosting-origin.json"
    receipt.write_text(json.dumps(origin, sort_keys=True, indent=2) + "\n")
    files = {path.name: path for path in directory.iterdir()}
    files[receipt.name] = receipt
    binding, _ = file_binding(files)
    return {"origin": origin, "directory": directory, "files": files, "binding": binding}


def workflow_context():
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_SERVER_URL") != "https://github.com":
        raise ValueError("hosted operations require GitHub.com Actions context")
    context = {k: os.environ.get(v, "") for k, v in {
        "repository": "GITHUB_REPOSITORY", "run_id": "GITHUB_RUN_ID", "attempt": "GITHUB_RUN_ATTEMPT",
        "sha": "GITHUB_SHA", "ref": "GITHUB_REF"}.items()}
    if (os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
            or os.environ.get("GITHUB_WORKFLOW_SHA") != context["sha"]
            or os.environ.get("GITHUB_WORKFLOW_REF") != context["repository"] + "/" + WORKFLOW + "@" + context["ref"]):
        raise ValueError("workflow is not the trusted manual archive entry point")
    return context


def memory_watch():
    if memory_available() < START_AVAILABLE:
        raise ValueError("archive requires at least 19 GiB MemAvailable")
    stop = threading.Event()

    def monitor():
        while not stop.wait(.5):
            try:
                available = memory_available()
            except (OSError, ValueError, StopIteration):
                available = 0
            if available < MIN_AVAILABLE:
                # Stop even while urllib/ZIP is blocked. Never seal a partially uploaded draft.
                print("archive stopped: cannot preserve 15 GiB available memory", file=sys.stderr, flush=True)
                os._exit(2)
    threading.Thread(target=monitor, daemon=True).start()
    return stop


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("verify", "publish"))
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--attempt", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--expected-binding")
    args = parser.parse_args()
    stop = None
    try:
        context = workflow_context()
        if args.operation == "publish" and not re.fullmatch(r"[0-9a-f]{64}", args.expected_binding or ""):
            raise ValueError("publish requires the read-only job's attachment binding")
        identity = checked_identity({"repository": context["repository"], "commit": args.commit,
            "tag_commit": args.commit, "tag": args.tag, "run_id": args.run_id, "attempt": args.attempt,
            "validator_revision": context["sha"]})
        stop = memory_watch()
        api = GitHub(allow_write=args.operation == "publish")
        with tempfile.TemporaryDirectory(prefix="rnet-hosted-archive-") as temporary:
            prepared = prepare_hosted(api, identity, context, Path(temporary) / "ready")
            if args.operation == "publish":
                result = publish(api, prepared, context, args.expected_binding)
                print("Candidate Release archived:", result["id"])
            else:
                output = os.environ.get("GITHUB_OUTPUT")
                if not output:
                    raise ValueError("GITHUB_OUTPUT is required")
                with open(output, "a") as stream:
                    stream.write("binding=" + prepared["binding"] + "\n")
                print("Read-only verification passed; attachment binding:", prepared["binding"])
            summary = os.environ.get("GITHUB_STEP_SUMMARY")
            if summary:
                with open(summary, "a") as stream:
                    stream.write(f"RNet candidate archive {args.operation}: `{identity['commit']}`\n\n"
                                 f"Qualification: {prepared['origin']['qualification_url']}\n\n"
                                 f"Attachment binding: `{prepared['binding']}`\n\n"
                                 "Not target-environment production certification.\n")
        return 0
    except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
        print("GitHub archive failed:", str(error), file=sys.stderr)
        return 1
    finally:
        if stop:
            stop.set()


if __name__ == "__main__":
    raise SystemExit(main())
