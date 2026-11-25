"""Version collection stays covered independently of gate timing tests."""

import json
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import evidence_source


class ToolchainSnapshotTests(unittest.TestCase):
    def test_configured_executables_are_single_arguments_and_environment_is_not_dumped(self):
        environment = {"DOTNET": "/fixture/sdk with spaces/dotnet",
                       "PROMTOOL": "/fixture/tools/promtool", "SECRET_FIXTURE": "not-for-evidence"}
        result = subprocess.CompletedProcess([], 0, stdout=" version fixture\n")
        with mock.patch.dict(os.environ, environment), mock.patch.object(
                evidence_source.subprocess, "run", return_value=result) as run:
            snapshot = evidence_source.toolchain_snapshot()
        self.assertEqual(snapshot["tools"]["dotnet"], {"exit_code": 0, "version": "version fixture"})
        for name in ("DOTNET", "PROMTOOL"):
            run.assert_any_call([environment[name], "--version"],
                                capture_output=True, text=True, timeout=10)
        self.assertNotIn(environment["SECRET_FIXTURE"], json.dumps(snapshot))
        self.assertTrue(snapshot["platform"])
        self.assertTrue(snapshot["machine"])

    def test_nonzero_exit_is_preserved(self):
        with mock.patch.object(evidence_source.subprocess, "run", return_value=
                               subprocess.CompletedProcess([], 7, stdout="partial output")):
            snapshot = evidence_source.toolchain_snapshot()
        self.assertEqual(snapshot["tools"]["cargo"], {"exit_code": 7, "version": "partial output"})

    def test_missing_or_timed_out_tool_does_not_hide_other_versions(self):
        for error in (FileNotFoundError("missing tool"), subprocess.TimeoutExpired(["rustc", "-Vv"], 10)):
            with self.subTest(error=type(error).__name__):
                def collect(command, **kwargs):
                    if command[0] == "rustc":
                        raise error
                    return subprocess.CompletedProcess(command, 0, stdout="available\n")
                with mock.patch.object(evidence_source.subprocess, "run", side_effect=collect):
                    snapshot = evidence_source.toolchain_snapshot()
                self.assertIn("unavailable", snapshot["tools"]["rustc"])
                self.assertNotIn("exit_code", snapshot["tools"]["rustc"])
                self.assertEqual(snapshot["tools"]["python"], {"exit_code": 0, "version": "available"})
