"""Release evidence must describe actual subprocess outcomes, not historical success."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "release_evidence.py"


class ReleaseEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not SCRIPT.exists():
            raise AssertionError("release evidence runner is not implemented")
        sys.path.insert(0, str(SCRIPT.parent))
        spec = importlib.util.spec_from_file_location("release_evidence", SCRIPT)
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rnet evidence ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        (self.root / ".gitignore").write_text("/target/\n")
        (self.root / "Cargo.lock").write_text("baseline lock\n")
        self.git("add", ".")
        self.git("-c", "user.name=Test", "-c", "user.email=test@invalid",
                 "commit", "-qm", "fixture")
        self.output = self.root / "target" / "evidence"

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args])

    def run_probe(self, code="print('gate output')", **options):
        return self.module.run_command(
            self.root, self.output, "fixture", [sys.executable, "-c", code],
            memory_reader=lambda: 20 * 1024**3, timeout=5, **options)

    def report(self):
        return json.loads((self.output / "report.json").read_text())

    def test_success_records_source_log_and_exit_status(self):
        self.assertEqual(self.run_probe(), 0)
        report = self.report()
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["exit_code"], 0)
        self.assertFalse(report["source_before"]["dirty"])
        self.assertEqual(report["source_before"], report["source_after"])
        self.assertEqual((self.output / "command.log").read_text(), "gate output\n")
        self.assertEqual(report["log_sha256"], self.module.sha256(self.output / "command.log"))

    def test_failure_and_missing_executable_are_not_success(self):
        self.assertNotEqual(self.run_probe("print('failure detail'); raise SystemExit(7)"), 0)
        self.assertEqual(self.report()["status"], "failed")
        self.assertEqual(self.report()["exit_code"], 7)
        self.assertIn("failure detail", (self.output / "command.log").read_text())
        self.output = self.root / "target" / "missing"
        result = self.module.run_command(
            self.root, self.output, "missing", ["/nonexistent/rnet-test-tool"],
            memory_reader=lambda: 20 * 1024**3)
        self.assertNotEqual(result, 0)
        self.assertEqual(self.report()["status"], "blocked")

    def test_dirty_development_run_is_labeled_and_release_refuses_it(self):
        (self.root / "new-source.rs").write_text("untracked input")
        self.assertEqual(self.run_probe(), 0)
        self.assertTrue(self.report()["source_before"]["dirty"])
        self.output = self.root / "target" / "release"
        self.assertNotEqual(self.run_probe(require_clean=True), 0)
        self.assertEqual(self.report()["status"], "blocked")
        self.assertIsNone(self.report()["exit_code"])

    def test_source_change_during_successful_command_invalidates_result(self):
        code = "from pathlib import Path; Path('Cargo.lock').write_text('changed')"
        self.assertNotEqual(self.run_probe(code), 0)
        self.assertEqual(self.report()["status"], "invalidated")
        self.assertNotEqual(self.report()["source_before"], self.report()["source_after"])

    def test_resource_refusal_and_running_pressure_stop_the_gate(self):
        for name, readings in [("start", [0]), ("running", [20 * 1024**3, 0])]:
            with self.subTest(name=name):
                self.output = self.root / "target" / name
                values = iter(readings)
                result = self.module.run_command(
                    self.root, self.output, "memory", [sys.executable, "-c", "import time; time.sleep(30)"],
                    memory_reader=lambda: next(values, 0), timeout=5)
                self.assertNotEqual(result, 0)
                self.assertEqual(self.report()["status"], "blocked")
                self.assertIn("memory", self.report()["reason"].lower())

    def test_timeout_and_resource_skip_output_are_not_passes(self):
        result = self.module.run_command(
            self.root, self.output, "timeout", [sys.executable, "-c", "import time; time.sleep(30)"],
            memory_reader=lambda: 20 * 1024**3, timeout=0.1)
        self.assertNotEqual(result, 0)
        self.assertEqual(self.report()["status"], "interrupted")
        self.output = self.root / "target" / "skip"
        self.assertNotEqual(self.run_probe("print('SKIPPED game soak smoke: memory guard')"), 0)
        self.assertEqual(self.report()["status"], "blocked")

    def test_existing_evidence_cannot_be_overwritten(self):
        self.assertEqual(self.run_probe(), 0)
        before = (self.output / "report.json").read_bytes()
        with self.assertRaises(FileExistsError):
            self.run_probe("raise SystemExit(9)")
        self.assertEqual((self.output / "report.json").read_bytes(), before)

    def test_output_must_be_ignored_and_cannot_follow_symlinks(self):
        self.output = self.root / "tracked-output"
        with self.assertRaises(ValueError):
            self.run_probe()
        self.assertFalse(self.output.exists())
        (self.root / "target").mkdir()
        (self.root / "target" / "link").symlink_to(self.root, target_is_directory=True)
        self.output = self.root / "target" / "link" / "report"
        with self.assertRaises(ValueError):
            self.run_probe()

    def test_summary_rejects_missing_tampered_and_mixed_revision_evidence(self):
        self.assertEqual(self.run_probe(), 0)
        path = self.output / "report.json"
        result = self.module.summarize([path], ["fixture"])
        self.assertEqual(result["status"], "passed")
        self.assertNotEqual(self.module.summarize([path], ["fixture", "other"])["status"], "passed")
        (self.output / "command.log").write_text("tampered")
        self.assertNotEqual(self.module.summarize([path], ["fixture"])["status"], "passed")
        (self.output / "command.log").unlink()
        self.assertNotEqual(self.module.summarize([path], ["fixture"])["status"], "passed")

    def test_summary_requires_same_source_and_unique_gates(self):
        self.assertEqual(self.run_probe(), 0)
        first = self.output / "report.json"
        self.assertNotEqual(self.module.summarize([first, first], ["fixture"])["status"], "passed")
        (self.root / "Cargo.lock").write_text("next revision")
        self.git("add", "Cargo.lock")
        self.git("-c", "user.name=Test", "-c", "user.email=test@invalid", "commit", "-qm", "next")
        self.output = self.root / "target" / "next"
        self.assertEqual(self.run_probe(), 0)
        second = self.output / "report.json"
        report = self.report()
        report["gate"] = "other"
        report["run_id"] = json.loads(first.read_text())["run_id"]
        second.write_text(json.dumps(report))
        self.assertNotEqual(self.module.summarize([first, second], ["fixture", "other"])["status"], "passed")

    def test_terminated_supervisor_seals_interrupted_result(self):
        code = f"""
import sys, signal
sys.path.insert(0, {str(SCRIPT.parent)!r})
import release_evidence as r
signal.signal(signal.SIGTERM, r.interrupt)
r.run_command({str(self.root)!r}, {str(self.output)!r}, 'signal',
              [sys.executable, '-c', "print('started', flush=True); import time; time.sleep(30)"],
              memory_reader=lambda: 20 * 1024**3)
"""
        child = subprocess.Popen([sys.executable, "-B", "-c", code], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 5
            log = self.output / "command.log"
            while time.monotonic() < deadline and child.poll() is None:
                if log.exists() and "started" in log.read_text():
                    break
                time.sleep(0.02)
            self.assertIsNone(child.poll(), "supervisor failed to start")
            child.terminate()
            child.communicate(timeout=5)
            self.assertEqual(self.report()["status"], "interrupted")
        finally:
            if child.poll() is None:
                child.kill()
            child.communicate(timeout=5)

    def test_package_seal_binds_build_start_and_detects_modified_files(self):
        package_module = __import__("package_evidence")
        before = self.module.source_snapshot(self.root)
        package = self.root / "target" / "package"
        package.mkdir(parents=True)
        (package / "librnet.a").write_bytes(b"fixture binary")
        package_module.seal(self.root, package, {"source": before, "environment": {"fixture": True}})
        info = json.loads((package / "BUILD-INFO.json").read_text())
        self.assertEqual(info["source"], before)
        files = [package / "librnet.a", package / "BUILD-INFO.json"]
        (package / "SHA256SUMS").write_text("".join(
            f"{self.module.sha256(path)}  ./{path.name}\n" for path in files))
        verified = package_module.verify(package, before)
        self.assertEqual(verified["manifest_sha256"], self.module.sha256(package / "SHA256SUMS"))
        (package / "librnet.a").write_bytes(b"modified")
        with self.assertRaises(ValueError):
            package_module.verify(package, before)
        (self.root / "Cargo.lock").write_text("changed during build")
        with self.assertRaises(ValueError):
            package_module.seal(self.root, package, {"source": before})

    def test_package_manifest_cannot_escape_or_omit_provenance(self):
        package_module = __import__("package_evidence")
        package = self.root / "target" / "package"
        package.mkdir(parents=True)
        for entry in ["../Cargo.lock", "/etc/passwd", "BUILD-INFO.json", "link"]:
            with self.subTest(entry=entry):
                (package / "SHA256SUMS").write_text(f"{'0' * 64}  {entry}\n")
                with self.assertRaises((ValueError, FileNotFoundError)):
                    package_module.verify(package, self.module.source_snapshot(self.root))

    def test_record_binds_environment_and_summary_checks_its_hash(self):
        self.assertEqual(self.run_probe(), 0)
        path = self.output / "environment.json"
        self.assertTrue(path.is_file())
        self.assertEqual(self.report()["environment_sha256"], self.module.sha256(path))
        path.write_text("{}")
        self.assertNotEqual(self.module.summarize([self.output / "report.json"], ["fixture"])["status"], "passed")

    def test_nonfinite_timeout_is_rejected_before_creating_output(self):
        for index, value in enumerate([0, -1, float("nan"), float("inf")]):
            self.output = self.root / "target" / f"deadline-{index}"
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.module.run_command(self.root, self.output, "deadline", ["true"], timeout=value)
            self.assertFalse(self.output.exists())

    def test_package_rejects_unlisted_files_and_symlinks(self):
        package_module = __import__("package_evidence")
        package = self.root / "target" / "package"
        package.mkdir(parents=True)
        before = self.module.source_snapshot(self.root)
        package_module.seal(self.root, package, {"source": before, "environment": {}})
        info = package / "BUILD-INFO.json"
        (package / "SHA256SUMS").write_text(f"{self.module.sha256(info)}  ./BUILD-INFO.json\n")
        extra = package / "unlisted-library"
        extra.write_bytes(b"not covered")
        with self.assertRaises(ValueError):
            package_module.verify(package, before)
        extra.unlink()
        extra.symlink_to(self.root / "Cargo.lock")
        with self.assertRaises(ValueError):
            package_module.verify(package, before)

    def test_timeout_reaps_descendant_processes(self):
        marker = self.root / "target" / "escaped"
        ready = self.root / "target" / "child-started"
        child = f"import time; from pathlib import Path; Path({str(ready)!r}).touch(); time.sleep(0.6); Path({str(marker)!r}).write_text('alive')"
        parent = f"import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', {child!r}]); time.sleep(30)"
        result = self.module.run_command(
            self.root, self.output, "descendants", [sys.executable, "-c", parent],
            memory_reader=lambda: 0 if ready.exists() else 20 * 1024**3, timeout=5)
        self.assertNotEqual(result, 0)
        self.assertTrue(ready.exists(), "test did not exercise a running descendant")
        time.sleep(0.7)
        self.assertFalse(marker.exists(), "timed-out gate left a descendant running")


if __name__ == "__main__":
    unittest.main()
