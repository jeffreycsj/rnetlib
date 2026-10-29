"""Offline verification must distrust status claims and historical absolute paths."""
import importlib
import json
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import unittest

from archive_fixture import fixture, write_json

ROOT = Path(__file__).resolve().parents[2]


class ArchiveValidationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not (ROOT / "scripts/archive_validation.py").exists():
            raise AssertionError("offline qualification verification is not implemented")
        sys.path.insert(0, str(ROOT / "scripts"))
        cls.module = importlib.import_module("archive_validation")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rnet evidence archive ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.evidence, self.archive, self.identity = fixture(self.root)

    def verify(self):
        return self.module.verify_inputs(self.evidence, self.archive, self.identity)

    def alter(self, name, mutate):
        path = self.evidence / name
        value = json.loads(path.read_text())
        mutate(value)
        write_json(path, value)

    def test_valid_records_can_move_without_rewriting_original_reports(self):
        originals = {str(path.relative_to(self.evidence)): path.read_bytes()
                     for path in self.evidence.rglob("*.json")}
        moved = self.root / "moved"
        shutil.move(self.evidence, moved)
        self.evidence = moved
        verified = self.verify()
        self.assertEqual(verified["source"]["revision"], self.identity["commit"])
        for name, data in originals.items():
            self.assertEqual((moved / name).read_bytes(), data)

    def test_wrong_identity_and_tag_binding_are_rejected(self):
        for key, bad in (("tag_commit", "f" * 40), ("commit", "f" * 40),
                         ("run_id", "102"), ("attempt", "1"), ("tag", "../tag"),
                         ("repository", "owner/repo/extra"), ("validator_revision", "main")):
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.module.verify_inputs(self.evidence, self.archive, {**self.identity, key: bad})

    def test_each_required_gate_is_mandatory(self):
        for gate in self.module.GATES:
            source = self.evidence / gate
            moved = self.root / "missing"
            shutil.move(source, moved)
            try:
                with self.subTest(gate=gate), self.assertRaises(ValueError):
                    self.verify()
            finally:
                shutil.move(moved, source)

    def test_record_changes_fail_even_when_summary_still_says_passed(self):
        path = self.evidence / "check/report.json"
        original = path.read_bytes()
        for field, value in (("status", "running"), ("exit_code", 1), ("exit_code", False),
                             ("gate", "go-race"), ("run_id", "101-1"), ("command", ["true"]),
                             ("reason", "resource unavailable"), ("source_after", {})):
            self.alter("check/report.json", lambda record: record.update({field: value}))
            try:
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    self.verify()
            finally:
                path.write_bytes(original)

    def test_dirty_source_is_rejected_even_when_all_records_agree(self):
        for path in self.evidence.glob("*/report.json"):
            record = json.loads(path.read_text())
            record["source_before"]["dirty"] = True
            record["source_after"]["dirty"] = True
            write_json(path, record)
        with self.assertRaises(ValueError):
            self.verify()

    def test_tampered_log_environment_and_extra_files_are_rejected(self):
        for name in ("check/command.log", "check/environment.json"):
            path = self.evidence / name
            original = path.read_bytes()
            path.write_bytes(b"tampered")
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.verify()
            path.write_bytes(original)
        (self.evidence / "unexpected").write_text("not part of the evidence contract")
        with self.assertRaises(ValueError):
            self.verify()

    def test_hashed_resource_skip_cannot_be_reported_as_a_pass(self):
        log = self.evidence / "check/command.log"
        log.write_text("SKIPPED game probe smoke: startup needs 19 GiB available")
        self.alter("check/report.json", lambda record: record.update(log_sha256=self.module.sha256(log)))
        with self.assertRaisesRegex(ValueError, "resource"):
            self.verify()

    def test_truncated_or_falsely_complete_summary_is_rejected(self):
        path = self.evidence / "summary.json"
        original = path.read_bytes()
        for field, value in (("required", ["check"]), ("reports", []), ("source", {}),
                             ("problems", ["missing gate"]), ("schema", 2)):
            self.alter("summary.json", lambda summary: summary.update({field: value}))
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.verify()
            path.write_bytes(original)

    def test_changed_sdk_or_checksum_cannot_reuse_previous_pass(self):
        checksum = Path(str(self.archive) + ".sha256")
        original = checksum.read_bytes()
        checksum.write_text("0" * 64 + "  linux-x86_64.tar.gz\n")
        with self.assertRaises(ValueError):
            self.verify()
        checksum.write_bytes(original)
        with self.archive.open("ab") as stream:
            stream.write(b"changed")
        with self.assertRaises(ValueError):
            self.verify()

    def test_recomputed_outer_hashes_do_not_hide_broken_inner_sdk_manifest(self):
        (self.root / "linux-x86_64/librnet.a").write_bytes(b"changed SDK contents")
        with tarfile.open(self.archive, "w:gz", format=tarfile.GNU_FORMAT) as stream:
            stream.add(self.root / "linux-x86_64", arcname="linux-x86_64")
        digest = self.module.sha256(self.archive)
        Path(str(self.archive) + ".sha256").write_text(f"{digest}  {self.archive.name}\n")
        for gate in ("package", "package-smoke"):
            self.alter(f"{gate}/report.json", lambda record: record["package"].update(archive_sha256=digest))
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.verify()
