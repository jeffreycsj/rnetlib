"""Bounded extraction of GitHub Actions ZIPs as data, with no executable metadata."""
import stat
import struct
import zipfile

from archive_io import MAX_BYTES, MAX_ENTRIES, MAX_MEMBER, no_symlinks, path_parts, regular_file


def _directory_limit(path):
    # Bound the central directory before ZipFile allocates one object per member.
    # Accept ordinary and ZIP64 end records, but not multipart/self-extracting archives.
    size = path.stat().st_size
    with path.open("rb") as stream:
        stream.seek(max(0, size - 65557))
        tail = stream.read(65557)
        at = tail.rfind(b"PK\x05\x06")
        if at < 0 or len(tail) - at < 22:
            raise ValueError("ZIP end record missing")
        _, disk, start, count_disk, count, central, offset, comment = struct.unpack_from("<4s4H2LH", tail, at)
        end = size - len(tail) + at
        if at + 22 + comment != len(tail) or disk or start:
            raise ValueError("multipart or malformed ZIP")
        locator = b""
        if end >= 20:
            stream.seek(end - 20)
            locator = stream.read(20)
        # zipfile honors a ZIP64 locator even when ordinary fields are not sentinels.
        has_zip64 = locator.startswith(b"PK\x06\x07")
        needs_zip64 = count == 65535 or count_disk == 65535 or central == 0xffffffff or offset == 0xffffffff
        if needs_zip64 and not has_zip64:
            raise ValueError("ZIP64 locator missing")
        if has_zip64:
            magic, disk64, location, disks = struct.unpack("<4sLQL", locator)
            if magic != b"PK\x06\x07" or disk64 or disks != 1 or location + 56 > end - 20:
                raise ValueError("invalid ZIP64 locator")
            stream.seek(location)
            record = struct.unpack("<4sQ2H2L4Q", stream.read(56))
            magic, length, _, _, disk, start, count_disk, count, central, offset = record
            # Fixed-size records keep prechecks aligned with older Python parsers
            # that locate ZIP64 by distance from the locator, ignoring its offset.
            if (magic != b"PK\x06\x06" or length != 44
                    or location + 12 + length != end - 20 or disk or start):
                raise ValueError("invalid ZIP64 directory")
        if count_disk != count or count > MAX_ENTRIES or central > 16 * 1024**2 or offset + central > end:
            raise ValueError("ZIP directory limit exceeded")
        return count


def extract_zip(path, destination):
    path = regular_file(path, MAX_BYTES)
    count = _directory_limit(path)
    destination = no_symlinks(destination)
    destination.mkdir(parents=True, exist_ok=False)
    seen, total = set(), 0
    with zipfile.ZipFile(path) as archive:
        if len(archive.infolist()) != count:
            raise ValueError("ZIP directory entry count mismatch")
        for member in archive.infolist():
            # orig_filename preserves NULs that zipfile's normalized name would hide.
            name = member.orig_filename
            directory = name.endswith("/")
            name = name[:-1] if directory else name
            parts = path_parts(name)
            kind = stat.S_IFMT(member.external_attr >> 16)
            if (name in seen or member.flag_bits & 1
                    or member.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
                    or kind not in (0, stat.S_IFDIR if directory else stat.S_IFREG)
                    or not 0 <= member.file_size <= MAX_MEMBER):
                raise ValueError("unsupported ZIP entry or size")
            seen.add(name)
            total += member.file_size
            if total > MAX_BYTES:
                raise ValueError("expanded ZIP limit exceeded")
            target = destination.joinpath(*parts)
            if directory:
                if member.file_size:
                    raise ValueError("directory contains data")
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(member) as incoming, target.open("xb") as outgoing:
                copied = 0
                for block in iter(lambda: incoming.read(65536), b""):
                    copied += len(block)
                    if copied > member.file_size:
                        raise ValueError("ZIP entry expanded beyond declared size")
                    outgoing.write(block)
                if copied != member.file_size:
                    raise ValueError("truncated ZIP entry")
