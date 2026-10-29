"""Bounded, data-only readers for untrusted qualification archives.

Extraction requires a new directory under caller-owned scratch space. No link,
device, executable permission, owner or timestamp from an archive is restored.
Callers must not allow concurrent writers in the input or scratch directories.
"""
import gzip
import json
import math
import os
from pathlib import Path
import stat
import tarfile

MAX_BYTES = 1024**3
MAX_MEMBER = 256 * 1024**2
MAX_ENTRIES = 10000


def no_symlinks(path):
    path = Path(path).absolute()
    if any(parent.is_symlink() for parent in (path, *path.parents)):
        raise ValueError("archive paths must not traverse symlinks")
    return path


def regular_file(path, maximum=MAX_MEMBER):
    path = no_symlinks(path)
    info = path.stat()
    if not stat.S_ISREG(info.st_mode) or info.st_size > maximum:
        raise ValueError("expected a bounded regular file")
    return path


def read_json(path, maximum=1024**2):
    """Reject duplicate keys, nonfinite numbers and oversized metadata."""
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result

    def constant(_):
        raise ValueError("nonfinite JSON value")

    def number(raw):
        value = float(raw)
        if not math.isfinite(value):
            raise ValueError("nonfinite JSON value")
        return value

    with regular_file(path, maximum).open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("JSON size limit exceeded")
    try:
        value = json.loads(data, object_pairs_hook=pairs, parse_constant=constant, parse_float=number)
    except RecursionError as error:
        raise ValueError("JSON nesting is too deep") from error
    if not isinstance(value, dict):
        raise ValueError("JSON record must be an object")
    return value


def path_parts(name):
    parts = name.split("/")
    if (not name or len(name) > 1024 or "\\" in name
            or any(ord(char) < 32 or ord(char) == 127 for char in name)
            or any(part in ("", ".", "..") for part in parts)):
        raise ValueError("noncanonical archive path")
    return parts


def inventory(root):
    """Bound both traversal and bytes; reject links and special files, even unlisted ones."""
    root = no_symlinks(root)
    if not root.is_dir():
        raise ValueError("inventory root must be a directory")
    files, pending, count, total = set(), [root], 0, 0
    while pending:
        with os.scandir(pending.pop()) as entries:
            for entry in entries:
                count += 1
                if count > MAX_ENTRIES:
                    raise ValueError("inventory entry limit exceeded")
                name = Path(entry.path).relative_to(root).as_posix()
                path_parts(name)
                info = entry.stat(follow_symlinks=False)
                if stat.S_ISDIR(info.st_mode):
                    pending.append(Path(entry.path))
                elif stat.S_ISREG(info.st_mode):
                    total += info.st_size
                    if info.st_size > MAX_MEMBER or total > MAX_BYTES:
                        raise ValueError("inventory size limit exceeded")
                    files.add(name)
                else:
                    raise ValueError("inventory contains a link or special file")
    return files


class _LimitedReader:
    def __init__(self, stream, maximum):
        self.stream, self.remaining = stream, maximum

    def read(self, size):
        # The limit also covers tar padding and extended headers, not just files.
        data = self.stream.read(min(size, self.remaining + 1))
        self.remaining -= len(data)
        if self.remaining < 0:
            raise ValueError("expanded archive byte limit exceeded")
        return data


def extract_tar(archive, destination, *, max_bytes=MAX_BYTES,
                max_member=MAX_MEMBER, max_entries=MAX_ENTRIES):
    """Extract regular data only, with bounded decompression and exclusive file creation.

    Failure can leave partial scratch data; callers should use TemporaryDirectory.
    Existing destinations are never reused. Limits may be lowered by tests.
    """
    archive = regular_file(archive, MAX_BYTES)
    destination = no_symlinks(destination)
    destination.mkdir(parents=True, exist_ok=False)
    seen = set()
    with gzip.open(archive, "rb") as expanded:
        reader = _LimitedReader(expanded, max_bytes)
        with tarfile.open(fileobj=reader, mode="r|") as stream:
            for member in stream:
                name = member.name.removesuffix("/") if member.isdir() else member.name
                parts = path_parts(name)
                if name in seen or len(seen) >= max_entries:
                    raise ValueError("duplicate member or archive entry limit exceeded")
                seen.add(name)
                if (member.size < 0 or member.size > max_member or member.sparse is not None
                        or member.pax_headers):
                    raise ValueError("unsupported metadata or archive member size exceeded")
                path = destination.joinpath(*parts)
                if member.isdir() and member.size == 0:
                    path.mkdir(parents=True, exist_ok=True)
                elif member.type in (tarfile.REGTYPE, tarfile.AREGTYPE):
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with stream.extractfile(member) as source, path.open("xb") as target:
                        remaining = member.size
                        while remaining:
                            chunk = source.read(min(65536, remaining))
                            if not chunk:
                                raise ValueError("truncated archive member")
                            target.write(chunk)
                            remaining -= len(chunk)
                else:
                    raise ValueError("archive links and special files are forbidden")
        # Tar stops at end markers; finish gzip to verify its CRC/trailer and keep
        # any trailing compressed data under the same decompression budget.
        while reader.read(65536):
            pass
