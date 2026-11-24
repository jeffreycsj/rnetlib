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
            floor = 6 * 1024**3
            self.assertEqual(release_evidence.START_AVAILABLE, 19 * 1024**3)
            self.assertEqual(release_evidence.MIN_AVAILABLE, 15 * 1024**3)
            for available in (0, floor - 1):
                with self.subTest(available=available), mock.patch.object(
                        release_evidence, "memory_available", return_value=available):
                    with self.assertRaisesRegex(SystemExit, "CI requires 6 GiB") as failure:
                        exec(code, {})
                    self.assertIn(f"{available / 1024**3:.2f} GiB", str(failure.exception))
                    self.assertIn("RNET_LINUX_RUNNER", str(failure.exception))
            for available in (floor, int(14.71 * 1024**3)):
                with mock.patch.object(release_evidence, "memory_available", return_value=available):
                    exec(code, {})
            with mock.patch.object(release_evidence, "memory_available", side_effect=OSError("unreadable")):
                with self.assertRaises(OSError):
                    exec(code, {})

    def test_linux_requires_resource_tests_and_checks_memory_before_building(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        body = re.search(r"(?ms)^  linux:\n(.*?)(?=^  [\w-]+:|\Z)", workflow).group(1)
        self.assertIn('RNET_REQUIRE_RESOURCE_TESTS: "1"', body)
        self.assertIn("runs-on: ${{ vars.RNET_LINUX_RUNNER || 'ubuntu-latest' }}", body)
        self.assertIn('RNET_RESOURCE_PROFILE: "github-ci"', body)
        self.assertIn("available < 6 * 1024**3", body)
        self.assertLess(body.index("available < 6 * 1024**3"),
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
