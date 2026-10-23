"""Source identity and atomic records for local Linux release evidence.

Hashes identify inputs, not their trustworthiness. Ignored build outputs are not
source inputs; symlinks are hashed as links, never followed outside the checkout.
"""

import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE)


def source_snapshot(root):
    """Include tracked and nonignored untracked files, deletions and mode changes."""
    root = Path(root)
    revision = git(root, "rev-parse", "HEAD").decode().strip()
    dirty = bool(git(root, "status", "--porcelain", "--untracked-files=all"))
    paths = set(git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0"))
    digest = hashlib.sha256()
    for raw in sorted(paths - {b""}):
        path = root / os.fsdecode(raw)
        digest.update(len(raw).to_bytes(8, "big") + raw)
        if path.is_symlink():
            value = b"link:" + os.fsencode(os.readlink(path))
        elif path.is_file():
            value = f"{path.stat().st_mode & 0o777}:{sha256(path)}".encode()
        elif not path.exists():
            value = b"deleted"
        else:
            raise ValueError(f"unsupported source input (e.g. submodule): {path}")
        digest.update(len(value).to_bytes(8, "big") + value)
    return {"revision": revision, "dirty": dirty, "input_sha256": digest.hexdigest(),
            "cargo_lock_sha256": sha256(root / "Cargo.lock")}


def atomic_json(path, value):
    """Replace only a complete record, leaving the previous state on write failure."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    name = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as stream:
            name = stream.name
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if name and os.path.exists(name):
            os.unlink(name)


def output_directory(root, output):
    """Never overwrite evidence or let report creation dirty its own source inputs."""
    root, output = Path(root).resolve(), Path(output).absolute()
    for parent in [output, *output.parents]:
        if parent.is_symlink():
            raise ValueError("evidence output must not traverse symlinks")
    try:
        relative = output.relative_to(root)
    except ValueError as error:
        raise ValueError("evidence must be inside the checkout's ignored build output") from error
    if not relative.parts or git(root, "ls-files", "--", str(relative)):
        raise ValueError("evidence output overlaps source files")
    ignored = subprocess.run(["git", "-C", str(root), "check-ignore", "-q", "--", str(relative)])
    if ignored.returncode != 0:
        raise ValueError("evidence output must be git-ignored")
    output.mkdir(parents=True, exist_ok=False)
    return output


def toolchain_snapshot():
    """Allowlist versions only: no environment dump, tokens or private key data."""
    commands = {"rustc": ["rustc", "-Vv"], "cargo": ["cargo", "--version"],
                "go": ["go", "version"], "dotnet": [os.environ.get("DOTNET", "dotnet"), "--version"],
                "cc": ["cc", "--version"], "python": ["python3", "--version"]}
    result = {}
    for name, command in commands.items():
        try:
            value = subprocess.run(command, capture_output=True, text=True, timeout=10)
            result[name] = {"exit_code": value.returncode, "version": value.stdout.strip()}
        except (OSError, subprocess.TimeoutExpired) as error:
            result[name] = {"unavailable": str(error)}
    return {"platform": platform.platform(), "machine": platform.machine(), "tools": result}
