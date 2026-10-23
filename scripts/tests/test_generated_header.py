import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "generated_header", Path(__file__).resolve().parents[1] / "generated_header.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class GeneratedHeaderTests(unittest.TestCase):
    def test_cli_prints_structured_compiler_error_without_updating_header(self):
        with tempfile.TemporaryDirectory() as directory:
            cargo, target = Path(directory)/"cargo", Path(directory)/"rnet.h"
            cargo.write_text("""#!/usr/bin/python3
import json, sys
if sys.argv[1] == 'metadata':
    print(json.dumps({'packages': [{'name': 'rnet-ffi', 'id': 'ffi'}]}))
else:
    print(json.dumps({'reason': 'compiler-message', 'message': {'rendered': 'specific Rust compile error\\n'}}))
    print(json.dumps({'reason': 'build-finished', 'success': False}))
    sys.exit(101)
""")
            cargo.chmod(0o700)
            target.write_bytes(b"retained\n")
            result = subprocess.run([sys.executable, str(SPEC.origin), "--update", "--output", str(target)],
                                    env=dict(os.environ, CARGO=str(cargo)), capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertIn("specific Rust compile error", result.stderr)
            self.assertEqual(target.read_bytes(), b"retained\n")

    def test_cli_preserves_cargo_failure_detail_and_existing_header(self):
        with tempfile.TemporaryDirectory() as directory:
            cargo, target = Path(directory)/"cargo", Path(directory)/"rnet.h"
            cargo.write_text("#!/bin/sh\necho 'metadata unavailable for test' >&2\nexit 71\n")
            cargo.chmod(0o700)
            target.write_bytes(b"retained\n")
            result = subprocess.run([sys.executable, str(SPEC.origin), "--update", "--output", str(target)],
                                    env=dict(os.environ, CARGO=str(cargo)), capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 1)
            self.assertIn("metadata unavailable for test", result.stderr)
            self.assertEqual(target.read_bytes(), b"retained\n")

    def test_selects_exact_package_from_cargo_output(self):
        messages = [
            {"reason": "build-script-executed", "package_id": "other", "out_dir": "/wrong"},
            {"reason": "build-script-executed", "package_id": "ffi", "out_dir": "/custom target/a"},
            {"reason": "build-finished", "success": True},
        ]
        self.assertEqual(MODULE.header_from_messages(messages, "ffi"), Path("/custom target/a/rnet.h"))

    def test_missing_or_ambiguous_or_failed_build_is_rejected(self):
        record = {"reason": "build-script-executed", "package_id": "ffi", "out_dir": "/a"}
        for messages in [[], [record], [record, {"reason": "build-finished", "success": False}],
                         [record, dict(record, out_dir="/b"), {"reason": "build-finished", "success": True}]]:
            with self.subTest(messages=messages), self.assertRaises(RuntimeError):
                MODULE.header_from_messages(messages, "ffi")

    def test_check_detects_drift_without_overwriting(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = Path(directory)/"generated.h", Path(directory)/"rnet.h"
            source.write_bytes(b"new declaration\n")
            target.write_bytes(b"old declaration\n")
            self.assertFalse(MODULE.publish_header(source, target))
            self.assertEqual(target.read_bytes(), b"old declaration\n")

    def test_update_is_reproducible_and_check_is_read_only(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = Path(directory)/"generated.h", Path(directory)/"rnet.h"
            source.write_bytes(b"generated contract\n")
            self.assertTrue(MODULE.publish_header(source, target, update=True))
            before = target.stat().st_mtime_ns
            self.assertTrue(MODULE.publish_header(source, target))
            self.assertTrue(MODULE.publish_header(source, target, update=True))
            self.assertEqual(target.stat().st_mtime_ns, before)
            self.assertEqual(target.read_bytes(), source.read_bytes())

    def test_missing_source_never_replaces_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)/"rnet.h"
            target.write_bytes(b"retained\n")
            with self.assertRaises(FileNotFoundError):
                MODULE.publish_header(Path(directory)/"absent", target, update=True)
            self.assertEqual(target.read_bytes(), b"retained\n")
