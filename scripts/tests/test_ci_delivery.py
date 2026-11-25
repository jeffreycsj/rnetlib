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

    def test_packaged_csharp_smoke_cannot_write_into_the_sealed_sdk(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertNotIn("dotnet run --project dist/", workflow)
        script = (ROOT / "scripts/test-package.sh").read_text()
        copy = 'cp -R "${package_dir}/csharp/RNet.Smoke" "${test_dir}/RNet.Smoke"'
        run = '"${DOTNET:-dotnet}" run --project "${test_dir}/RNet.Smoke"'
        self.assertIn(copy, script)
        self.assertIn(run, script)
        self.assertLess(script.index(copy), script.index(run))
        self.assertLess(script.index(run), script.rindex('scripts/package_evidence.py verify'))
        self.assertIn('-p:RNetDll="${package_dir}/lib/RNet.dll"', script[script.index(run):])

    def test_sdk_upload_requires_supported_python_script_matrix(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        job = re.search(r"(?ms)^  python-compat:\n(.*?)(?=^  [\w-]+:|\Z)", workflow)
        self.assertIsNotNone(job, "SDK delivery needs cross-version script validation")
        body = job.group(1)
        self.assertIn('python-version: ["3.10", "3.14"]', body)
        self.assertIn("python-version: ${{ matrix.python-version }}", body)
        self.assertRegex(body, r"uses: actions/setup-python@[0-9a-f]{40}\b")
        self.assertIn("python3 -B -m unittest discover -s scripts/tests -v", body)
        self.assertIn("fail-fast: false", body)
        self.assertNotIn("continue-on-error", body)
        linux = re.search(r"(?ms)^  linux:\n(.*?)(?=^  [\w-]+:|\Z)", workflow).group(1)
        self.assertIn("needs: python-compat", linux)
        self.assertNotIn("if: always()", linux)

    def test_remote_workflow_actions_use_immutable_commits(self):
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            actions = re.findall(r"(?m)^\s*(?:-\s*)?uses:\s*(\S+)", path.read_text())
            for action in actions:
                if not action.startswith("./"):
                    with self.subTest(workflow=path.name, action=action):
                        self.assertRegex(action, r"^[\w.-]+/[\w./-]+@[0-9a-f]{40}$")

    def test_artifact_uploads_preserve_zip_and_refuse_replacement(self):
        for name in ("ci.yml", "release-qualification.yml"):
            workflow = (ROOT / ".github/workflows" / name).read_text()
            uploads = re.findall(
                r"(?ms)^        uses: actions/upload-artifact@[^\n]+\n(.*?)(?=^      - |\Z)", workflow)
            self.assertTrue(uploads, name)
            for body in uploads:
                with self.subTest(workflow=name, upload=body):
                    self.assertIn("archive: true", body)
                    self.assertIn("overwrite: false", body)
                    self.assertIn("if-no-files-found: error", body)
