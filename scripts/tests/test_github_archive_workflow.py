"""Protect the two-job permission boundary and reject unsafe CLI invocation contexts."""
import os
from pathlib import Path
import re
import unittest
from unittest import mock

from archive_fixture import fixture
from github_archive import workflow_context, memory_watch

ROOT = Path(__file__).resolve().parents[2]


class WorkflowTests(unittest.TestCase):
    def test_default_disabled_manual_two_job_approval_boundary(self):
        workflow = (ROOT / ".github/workflows/release-archive.yml").read_text()
        self.assertIn("workflow_dispatch:", workflow)
        self.assertNotIn("workflow_run:", workflow)
        self.assertIn("permissions: {}", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        verify = workflow.split("  verify:\n", 1)[1].split("  publish:\n", 1)[0]
        publish = workflow.split("  publish:\n", 1)[1]
        self.assertIn("vars.RNET_RELEASE_ARCHIVE_ENABLED == 'true'", verify)
        self.assertIn("github.event.repository.default_branch", verify)
        self.assertNotIn("contents: write", verify)
        self.assertNotIn("environment:", verify)
        self.assertIn("needs: verify", publish)
        self.assertIn("environment: rnet-release-archive", publish)
        self.assertIn("contents: write", publish)
        self.assertIn("needs.verify.outputs.binding", publish)
        for job in (verify, publish):
            self.assertIn("ref: ${{ github.workflow_sha }}", job)
            self.assertIn("persist-credentials: false", job)
            self.assertRegex(job, r"uses: actions/checkout@[0-9a-f]{40}")
            self.assertIn("--tag \"$CANDIDATE_TAG\"", job)
            self.assertNotIn("continue-on-error", job)
        for shell in re.findall(r"run: \|\n((?:          .*\n)+)", workflow):
            self.assertNotIn("${{", shell)
        self.assertNotIn("download-artifact@", workflow)

    def test_only_actual_manual_workflow_context_is_accepted(self):
        values = {"GITHUB_ACTIONS": "true", "GITHUB_SERVER_URL": "https://github.com",
                  "GITHUB_REPOSITORY": "example/netlib", "GITHUB_RUN_ID": "201",
                  "GITHUB_RUN_ATTEMPT": "1", "GITHUB_SHA": "a" * 40,
                  "GITHUB_REF": "refs/heads/main", "GITHUB_WORKFLOW_SHA": "a" * 40,
                  "GITHUB_EVENT_NAME": "workflow_dispatch",
                  "GITHUB_WORKFLOW_REF": "example/netlib/.github/workflows/release-archive.yml@refs/heads/main"}
        with mock.patch.dict(os.environ, values, clear=True):
            self.assertEqual(workflow_context()["run_id"], "201")
            for field in ("GITHUB_ACTIONS", "GITHUB_SERVER_URL", "GITHUB_WORKFLOW_SHA", "GITHUB_WORKFLOW_REF", "GITHUB_EVENT_NAME"):
                with self.subTest(field=field), mock.patch.dict(os.environ, {field: "invalid"}):
                    with self.assertRaises(ValueError): workflow_context()

    def test_low_or_unknown_memory_refuses_start(self):
        for available in (0, 19 * 1024**3 - 1):
            with mock.patch("github_archive.memory_available", return_value=available):
                with self.assertRaises(ValueError): memory_watch()
        with mock.patch("github_archive.memory_available", side_effect=OSError("unreadable")):
            with self.assertRaises(OSError): memory_watch()
