"""Exercise the real Make gate with deterministic external audit outcomes."""
import os
import re
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
CHECKS = [
    "audit --deny warnings",
    "deny --locked check",
    "audit --file fuzz/Cargo.lock --deny warnings",
    "deny --manifest-path fuzz/Cargo.toml --locked --config deny.toml check",
    "audit --file vendor/rnet-kcp/upstream.Cargo.lock --deny warnings",
]


class DependencyGateTests(unittest.TestCase):
    def test_ci_uses_the_same_gate_and_pinned_audit_tools(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        job = re.search(r"(?ms)^  supply-chain:\n(.*?)(?=^  [\w-]+:|\Z)", workflow)
        self.assertIsNotNone(job)
        body = job.group(1)
        self.assertIn("- run: make -j1 dependency-check", body)
        for tool, version in [("cargo-audit", "0.22.2"), ("cargo-deny", "0.20.2")]:
            self.assertIn(f"cargo install {tool} --version {version} --locked", body)
        self.assertNotIn("continue-on-error", body)
        self.assertNotIn("rustsec/audit-check", body)
        self.assertNotIn("EmbarkStudios/cargo-deny-action", body)

    def test_ncsa_is_only_allowed_for_the_approved_fuzzer_release(self):
        sections = re.split(r"(?m)(?=^\[)", (ROOT / "deny.toml").read_text())
        exceptions = [section for section in sections if '"NCSA"' in section]
        self.assertEqual(len(exceptions), 1)
        exception, = exceptions
        self.assertTrue(exception.startswith("[[licenses.exceptions]]"))
        self.assertRegex(exception, r'(?m)^name = "libfuzzer-sys"$')
        self.assertRegex(exception, r'(?m)^version = "=0\.4\.13"$')
        self.assertRegex(exception, r'(?m)^allow = \["NCSA"\]$')

    def run_gate(self, fail=""):
        with tempfile.TemporaryDirectory(prefix="rnet dependency gate ") as directory:
            root = Path(directory)
            cargo = root / "cargo"
            cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$RNET_TEST_CALLS"\n'
                             'test "$*" != "$RNET_TEST_FAIL"\n')
            cargo.chmod(0o755)
            calls = root / "calls"
            env = dict(os.environ, PATH=f"{root}{os.pathsep}{os.environ['PATH']}",
                       RNET_TEST_CALLS=str(calls), RNET_TEST_FAIL=fail, MAKEFLAGS="")
            result = subprocess.run(["make", "-j1", "dependency-check"], cwd=ROOT,
                                    env=env, capture_output=True, text=True, timeout=10)
            return result, calls.read_text().splitlines()

    def test_success_checks_both_locked_graphs_with_the_same_policy(self):
        result, calls = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, CHECKS)

    def test_each_failed_audit_stops_the_gate(self):
        for index, command in enumerate(CHECKS):
            with self.subTest(command=command):
                result, calls = self.run_gate(fail=command)
                self.assertNotEqual(result.returncode, 0, "failed audit was accepted")
                self.assertEqual(calls, CHECKS[:index + 1])
