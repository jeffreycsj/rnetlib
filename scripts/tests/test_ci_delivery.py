"""Keep development SDK uploads behind the actual archived-package gate."""
from pathlib import Path
import re
import sys
import textwrap
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]


class CIDeliveryTests(unittest.TestCase):
    def test_actual_ci_preflight_rejects_low_or_unknown_memory(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        block = re.search(r"python3 -B - <<'PY'\n(.*?)\n          PY", workflow, re.S)
        self.assertIsNotNone(block)
        code = compile(textwrap.dedent(block.group(1)), "ci-memory-preflight", "exec")
        with mock.patch.object(sys, "path", [str(ROOT / "scripts"), *sys.path]):
            import release_evidence
            floor = 19 * 1024**3
            self.assertEqual(release_evidence.START_AVAILABLE, floor)
            for available in (0, floor - 1):
                with self.subTest(available=available), mock.patch.object(
                        release_evidence, "memory_available", return_value=available):
                    with self.assertRaisesRegex(SystemExit, "CI requires 19 GiB"):
                        exec(code, {})
            with mock.patch.object(release_evidence, "memory_available", return_value=floor):
                exec(code, {})
            with mock.patch.object(release_evidence, "memory_available", side_effect=OSError("unreadable")):
                with self.assertRaises(OSError):
                    exec(code, {})

    def test_linux_requires_resource_tests_and_checks_memory_before_building(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        body = re.search(r"(?ms)^  linux:\n(.*?)(?=^  [\w-]+:|\Z)", workflow).group(1)
        self.assertIn('RNET_REQUIRE_RESOURCE_TESTS: "1"', body)
        self.assertIn("memory_available() < START_AVAILABLE", body)
        self.assertLess(body.index("memory_available() < START_AVAILABLE"),
                        body.index("- run: cargo"))

    def test_linux_upload_requires_archived_package_smoke(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        job = re.search(r"(?ms)^  linux:\n(.*?)(?=^  [\w-]+:|\Z)", workflow)
        self.assertIsNotNone(job)
        body = job.group(1)
        self.assertIn("- run: make package-smoke", body)
        self.assertLess(body.index("- run: make package\n"),
                        body.index("- run: make package-smoke"))
        self.assertLess(body.index("- run: make package-smoke"),
                        body.index("uses: actions/upload-artifact@"))
        self.assertNotIn("continue-on-error", body)
        self.assertNotIn("if: always()", body)
