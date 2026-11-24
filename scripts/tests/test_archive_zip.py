"""Reject unsafe ZIP data before it can become a Release attachment."""
from pathlib import Path
import stat
import struct
import tempfile
import unittest
from unittest import mock
import zipfile

from archive_fixture import fixture  # sets the source script import path
from archive_zip import extract_zip


class ZipTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.zip = self.root / "input.zip"
        self.out = self.root / "out"

    def test_regular_and_zip64_member_data_without_executable_permissions(self):
        with zipfile.ZipFile(self.zip, "w") as z:
            z.writestr("nested/", b"")
            info = zipfile.ZipInfo("nested/data"); info.external_attr = (stat.S_IFREG | 0o755) << 16
            with z.open(info, "w", force_zip64=True) as f: f.write(b"opaque")
        extract_zip(self.zip, self.out)
        self.assertEqual((self.out / "nested/data").read_bytes(), b"opaque")
        self.assertEqual((self.out / "nested/data").stat().st_mode & 0o111, 0)
        with self.assertRaises(FileExistsError): extract_zip(self.zip, self.out)

    def test_traversal_links_and_special_entries_are_rejected(self):
        for name, mode in (("../escape", stat.S_IFREG), ("/absolute", stat.S_IFREG),
                           ("a\\b", stat.S_IFREG), ("link", stat.S_IFLNK), ("fifo", stat.S_IFIFO)):
            with self.subTest(name=name):
                with zipfile.ZipFile(self.zip, "w") as z:
                    info = zipfile.ZipInfo(name); info.external_attr = (mode | 0o644) << 16
                    z.writestr(info, b"data")
                with tempfile.TemporaryDirectory(dir=self.root) as d:
                    with self.assertRaises(ValueError): extract_zip(self.zip, Path(d) / "out")
        self.assertFalse((self.root / "escape").exists())

    def test_directory_count_and_crc_tampering_fail(self):
        with zipfile.ZipFile(self.zip, "w", zipfile.ZIP_STORED) as z: z.writestr("data", b"opaque")
        original = self.zip.read_bytes()
        changed = bytearray(original)
        at = changed.rfind(b"PK\x05\x06")
        struct.pack_into("<HH", changed, at + 8, 10001, 10001)
        self.zip.write_bytes(changed)
        with self.assertRaises(ValueError): extract_zip(self.zip, self.out)
        self.assertFalse(self.out.exists())
        self.zip.write_bytes(original.replace(b"opaque", b"BROKEN"))
        with self.assertRaises(zipfile.BadZipFile): extract_zip(self.zip, self.out)

    def test_large_directory_rejected_before_zipfile_allocation(self):
        with zipfile.ZipFile(self.zip, "w") as z: z.writestr("data", b"x")
        changed = bytearray(self.zip.read_bytes())
        at = changed.rfind(b"PK\x05\x06")
        struct.pack_into("<L", changed, at + 12, 17 * 1024**2)
        self.zip.write_bytes(changed)
        with self.assertRaises(ValueError): extract_zip(self.zip, self.out)
        self.assertFalse(self.out.exists())

    def test_zip64_limit_cannot_hide_behind_small_ordinary_end_record(self):
        with zipfile.ZipFile(self.zip, "w") as z: z.writestr("data", b"x")
        raw = self.zip.read_bytes(); at = raw.rfind(b"PK\x05\x06")
        z64 = struct.pack("<4sQ2H2L4Q", b"PK\x06\x06", 44, 45, 45, 0, 0, 1, 1, 17 * 1024**2, 0)
        locator = struct.pack("<4sLQL", b"PK\x06\x07", 0, at, 1)
        self.zip.write_bytes(raw[:at] + z64 + locator + raw[at:])
        with mock.patch("archive_zip.zipfile.ZipFile", side_effect=AssertionError("unbounded allocation")):
            with self.assertRaises(ValueError): extract_zip(self.zip, self.out)

    def test_zip64_directory_is_supported(self):
        # Force a real ZIP64 directory without allocating a multi-gigabyte fixture.
        with mock.patch.object(zipfile, "ZIP64_LIMIT", 1):
            with zipfile.ZipFile(self.zip, "w") as archive:
                archive.writestr("data", b"opaque")
        self.assertIn(b"PK\x06\x06", self.zip.read_bytes())
        extract_zip(self.zip, self.out)
        self.assertEqual((self.out / "data").read_bytes(), b"opaque")

    def test_extensible_zip64_directory_rejected_before_parser(self):
        with zipfile.ZipFile(self.zip, "w") as archive:
            archive.writestr("data", b"x")
        raw = self.zip.read_bytes(); at = raw.rfind(b"PK\x05\x06")
        central, offset = struct.unpack_from("<LL", raw, at + 12)
        record = struct.pack("<4sQ2H2L4Q", b"PK\x06\x06", 100, 45, 45, 0, 0, 1, 1, central, offset)
        # Older zipfile versions read the final 56 bytes rather than the locator offset.
        extension = struct.pack("<4sQ2H2L4Q", b"PK\x06\x06", 44, 45, 45, 0, 0, 1, 1, 17 * 1024**2, 0)
        locator = struct.pack("<4sLQL", b"PK\x06\x07", 0, at, 1)
        self.zip.write_bytes(raw[:at] + record + extension + locator + raw[at:])
        with mock.patch("archive_zip.zipfile.ZipFile", side_effect=AssertionError("ambiguous parser")):
            with self.assertRaises(ValueError): extract_zip(self.zip, self.out)
