"""Round-trip actual files through the public offline archive CLI."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from archive_fixture import fixture, write_json

SCRIPT = Path(__file__).resolve().parents[1] / "release_archive.py"


class ReleaseArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rnet release archive ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.evidence, self.sdk, identity = fixture(self.root)
        self.identity = self.root / "identity.json"
        write_json(self.identity, identity)
        self.output = self.root / "output"

    def cli(self, operation, *args):
        return subprocess.run([sys.executable, "-B", str(SCRIPT), operation,
                               "--identity", str(self.identity), *map(str, args)],
                              capture_output=True, text=True, timeout=10)

    def build(self, output=None):
        return self.cli("prepare", "--evidence", self.evidence, "--sdk", self.sdk,
                        "--output", output or self.output)

    def require_build(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_archive_round_trips_after_original_inputs_are_gone(self):
        self.require_build()
        shutil.rmtree(self.evidence)
        self.sdk.unlink()
        result = self.cli("verify", "--archive-dir", self.output)
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = json.loads((self.output / "archive-manifest.json").read_text())
        self.assertEqual(manifest["scope"], "offline consistency only; not origin or approval authentication")
        self.assertEqual(set(manifest["artifacts"]), {"evidence.tar.gz", "evidence.tar.gz.sha256",
                                                       "linux-x86_64.tar.gz", "linux-x86_64.tar.gz.sha256"})

    def test_repeated_preparation_is_reproducible_but_never_overwrites(self):
        self.require_build()
        files = {path.name: path.read_bytes() for path in self.output.iterdir()}
        result = self.build(self.root / "second")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(files, {path.name: path.read_bytes() for path in (self.root / "second").iterdir()})
        self.assertNotEqual(self.build().returncode, 0)
        self.assertEqual(files, {path.name: path.read_bytes() for path in self.output.iterdir()})

    def test_incomplete_evidence_creates_no_ready_archive(self):
        (self.evidence / "go-race/report.json").unlink()
        self.assertNotEqual(self.build().returncode, 0)
        self.assertFalse(self.output.exists())

    def test_changed_attachment_and_unlisted_files_fail_verification(self):
        self.require_build()
        for name in ("evidence.tar.gz", "linux-x86_64.tar.gz", "evidence.tar.gz.sha256"):
            path = self.output / name
            original = path.read_bytes()
            path.write_bytes(b"tampered")
            result = self.cli("verify", "--archive-dir", self.output)
            self.assertNotEqual(result.returncode, 0, name)
            path.write_bytes(original)
        (self.output / "unlisted").write_text("extra")
        self.assertNotEqual(self.cli("verify", "--archive-dir", self.output).returncode, 0)

    def test_changed_manifest_identity_is_not_a_trusted_expected_identity(self):
        self.require_build()
        path = self.output / "archive-manifest.json"
        manifest = json.loads(path.read_text())
        manifest["identity"]["repository"] = "other/repository"
        write_json(path, manifest)
        self.assertNotEqual(self.cli("verify", "--archive-dir", self.output).returncode, 0)

    def test_unknown_manifest_claims_are_not_echoed_as_verified(self):
        self.require_build()
        path = self.output / "archive-manifest.json"
        manifest = json.loads(path.read_text())
        manifest["production_certified"] = True
        write_json(path, manifest)
        self.assertNotEqual(self.cli("verify", "--archive-dir", self.output).returncode, 0)
