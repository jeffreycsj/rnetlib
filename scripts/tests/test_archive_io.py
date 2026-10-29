"""Exercise hostile archives without executing their contents or large allocations."""
import importlib
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ArchiveIOTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not (ROOT / "scripts/archive_io.py").exists():
            raise AssertionError("bounded archive reader is not implemented")
        sys.path.insert(0, str(ROOT / "scripts"))
        cls.module = importlib.import_module("archive_io")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rnet archive test ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def archive(self, members):
        path = self.root / "input.tar.gz"
        with tarfile.open(path, "w:gz") as stream:
            for name, kind, payload in members:
                entry = tarfile.TarInfo(name)
                entry.type = kind
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    entry.linkname = "../../outside"
                entry.size = len(payload)
                stream.addfile(entry, io.BytesIO(payload))
        return path

    def test_normal_archive_and_zero_length_file(self):
        path = self.archive([("sdk/", tarfile.DIRTYPE, b""),
                             ("sdk/empty", tarfile.REGTYPE, b""),
                             ("sdk/binary", tarfile.REGTYPE, b"\x00\xff")])
        self.module.extract_tar(path, self.root / "output")
        self.assertEqual((self.root / "output/sdk/binary").read_bytes(), b"\x00\xff")
        self.assertEqual(self.module.inventory(self.root / "output"), {"sdk/empty", "sdk/binary"})

    def test_traversal_links_devices_and_duplicates_are_rejected(self):
        cases = [[(name, tarfile.REGTYPE, b"x")] for name in
                 ("../outside", "/outside", "sdk/../outside", "sdk//file", "sdk/./file", "sdk\\file")]
        cases += [[("sdk/bad", kind, b"")] for kind in
                  (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE, tarfile.CHRTYPE)]
        cases += [[("same", tarfile.REGTYPE, b"one"), ("same", tarfile.REGTYPE, b"two")]]
        for index, members in enumerate(cases):
            with self.subTest(members=members), self.assertRaises(ValueError):
                self.module.extract_tar(self.archive(members), self.root / f"output-{index}")
        self.assertFalse((self.root / "outside").exists())

    def test_limits_apply_to_content_headers_and_member_count(self):
        path = self.archive([("a", tarfile.REGTYPE, b"x" * 32), ("b", tarfile.REGTYPE, b"x" * 32)])
        for index, limits in enumerate(({"max_member": 31}, {"max_bytes": 63}, {"max_entries": 1})):
            with self.subTest(limits=limits), self.assertRaises(ValueError):
                self.module.extract_tar(path, self.root / f"output-{index}", **limits)

    def test_existing_directory_and_symlink_ancestors_are_not_written(self):
        path = self.archive([("file", tarfile.REGTYPE, b"replace")])
        output = self.root / "existing"
        output.mkdir()
        (output / "file").write_bytes(b"original")
        with self.assertRaises(FileExistsError):
            self.module.extract_tar(path, output)
        (self.root / "link").symlink_to(output, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.module.extract_tar(path, self.root / "link/new")
        self.assertEqual((output / "file").read_bytes(), b"original")

    def test_inventory_and_json_reject_ambiguous_input(self):
        data = self.root / "data.json"
        for value in ('{"a":1,"a":2}', '{"a":NaN}', '{"a":1e999}', '[]', '{',
                      '{"a":' + '[' * 2000 + '0' + ']' * 2000 + '}'):
            data.write_text(value)
            with self.subTest(value=value[:40]), self.assertRaises(ValueError):
                self.module.read_json(data)
        data.write_text('{"a": 1}')
        self.assertEqual(self.module.read_json(data), {"a": 1})
        with self.assertRaises(ValueError):
            self.module.read_json(data, maximum=2)
        (self.root / "link").symlink_to(data)
        with self.assertRaises(ValueError):
            self.module.inventory(self.root)

    def test_corrupt_gzip_footer_is_not_ignored_after_tar_end(self):
        import gzip
        # Tar may stop at its end markers before GzipFile checks its trailer.
        raw = io.BytesIO()
        with tarfile.open(fileobj=raw, mode="w") as stream:
            stream.addfile(tarfile.TarInfo("empty"))
        data = bytearray(gzip.compress(raw.getvalue()))
        data[-8] ^= 1
        path = self.root / "corrupt.tar.gz"
        path.write_bytes(data)
        with self.assertRaises((ValueError, OSError)):
            self.module.extract_tar(path, self.root / "output")
