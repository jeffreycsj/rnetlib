"""Check the consumer gate's isolation and fail-closed behavior without compiling."""

import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("check_rust_consumer", ROOT / "scripts/check_rust_consumer.py")
CONSUMER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONSUMER)


class RustConsumerTests(unittest.TestCase):
    def test_real_temporary_workspace_has_no_profile_or_patch_workaround(self):
        calls = []

        def inspect(command, *, cwd, env, check):
            manifest = (cwd / "Cargo.toml").read_text()
            self.assertIn("[workspace]", manifest)
            self.assertNotIn("[profile", manifest)
            self.assertNotIn("[patch", manifest)
            self.assertTrue((cwd / "Cargo.lock").is_file())
            self.assertTrue((cwd / "tests/boundaries.rs").is_file())
            self.assertTrue((cwd / "tests/kcp_engine.rs").is_file())
            self.assertFalse(cwd.is_relative_to(ROOT))
            self.assertTrue(check)
            self.assertEqual(env["CARGO_BUILD_JOBS"], "1")
            if "--release" in command:
                self.assertEqual(env["CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS"], "true")
            calls.append((command, cwd))

        with patch.object(CONSUMER.subprocess, "run", side_effect=inspect):
            CONSUMER.main()
        self.assertEqual(len(calls), 2)
        self.assertNotIn("--release", calls[0][0])
        self.assertIn("--release", calls[1][0])
        self.assertFalse(calls[0][1].exists(), "temporary project must be cleaned up")

    def test_failed_consumer_test_is_not_hidden_by_second_profile(self):
        with patch.object(CONSUMER.subprocess, "run", side_effect=subprocess.CalledProcessError(101, "cargo")) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                CONSUMER.main()
        self.assertEqual(run.call_count, 1)

    def test_ci_and_local_check_both_run_the_consumer_gate(self):
        self.assertIn("- run: make rust-consumer-check", (ROOT / ".github/workflows/ci.yml").read_text())
        self.assertIn("\t$(MAKE) rust-consumer-check", (ROOT / "Makefile").read_text())

    def test_fork_identity_and_license_are_retained(self):
        provenance = (ROOT / "vendor/rnet-kcp/UPSTREAM.md").read_text()
        identity = (ROOT / "vendor/rnet-kcp/upstream.Cargo.lock").read_text()
        self.assertIn('name = "kcp"', identity)
        self.assertIn('version = "0.6.0"', identity)
        checksum = "9be8f8058681ab8b7e4959a1ee88a4446f43b7c787f174d9d1b32ba493737a59"
        self.assertIn(checksum, identity)
        self.assertIn(checksum, provenance)
        self.assertIn("Copyright (c) 2017 Zhang Cheng", (ROOT / "vendor/rnet-kcp/LICENSE").read_text())
        self.assertIn("vendor/rnet-kcp/LICENSE", (ROOT / "scripts/package.sh").read_text())
