"""Publish/check the header emitted by Cargo without guessing build directory hashes."""

import argparse
import difflib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def header_from_messages(messages, package_id):
    outputs = {Path(m["out_dir"]) / "rnet.h" for m in messages
               if m.get("reason") == "build-script-executed" and m.get("package_id") == package_id}
    finished = [m.get("success") for m in messages if m.get("reason") == "build-finished"]
    if finished != [True] or len(outputs) != 1:
        raise RuntimeError("Cargo did not report one successful FFI header build")
    return outputs.pop()


def publish_header(source, destination, update=False):
    generated = source.read_bytes()
    current = destination.read_bytes() if destination.exists() else None
    if generated == current:
        return True
    if not update:
        diff = difflib.unified_diff((current or b"").decode().splitlines(True),
                                    generated.decode().splitlines(True),
                                    fromfile=str(destination), tofile="Cargo-generated rnet.h")
        sys.stderr.writelines(diff)
        return False
    # Only explicit update may change the source tree. Atomic replace preserves the
    # previous complete header if writing fails; identical output preserves mtime.
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
            temporary = Path(output.name)
            output.write(generated)
        temporary.chmod(0o644)
        temporary.replace(destination)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    return True


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--update", action="store_true")
    parser.add_argument("--output", type=Path, default=root / "include/rnet.h")
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--release", action="store_true")
    args = parser.parse_args()
    cargo = os.environ.get("CARGO", "cargo")
    metadata = subprocess.run([cargo, "metadata", "--locked", "--no-deps", "--format-version=1"],
                              cwd=root, check=True, capture_output=True, text=True)
    package_id, = [p["id"] for p in json.loads(metadata.stdout)["packages"] if p["name"] == "rnet-ffi"]
    command = [cargo, "build", "--locked", "-p", "rnet-ffi", "--message-format=json"]
    if args.release:
        command.append("--release")
    if args.target_dir is not None:
        command += ["--target-dir", str(args.target_dir)]
    result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, text=True)
    messages = [json.loads(line) for line in result.stdout.splitlines() if line]
    # Cargo's JSON mode puts rustc diagnostics on stdout along with artifact metadata.
    # Preserve those details before turning an unsuccessful build into a CLI error.
    for message in messages:
        if message.get("reason") == "compiler-message":
            rendered = message.get("message", {}).get("rendered")
            if rendered:
                print(rendered.rstrip(), file=sys.stderr)
    result.check_returncode()
    header = header_from_messages(messages, package_id)
    if not publish_header(header, args.output, update=args.update):
        print("rnet.h is stale; run make header-update and review the ABI/documentation diff", file=sys.stderr)
        return 1
    print(f"{'Updated' if args.update else 'Verified'} {args.output}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr.rstrip(), file=sys.stderr)
        print(f"header generation failed: {error}", file=sys.stderr)
        sys.exit(1)
