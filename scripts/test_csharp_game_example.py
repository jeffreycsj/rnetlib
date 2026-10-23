"""Run the real C# executable against the native library (no NuGet test packages)."""
import os
from pathlib import Path
import re
import subprocess
import unittest


class CSharpGameExampleTests(unittest.TestCase):
    def run_example(self, *args):
        root = Path(__file__).resolve().parents[1]
        assembly = os.environ.get("RNET_CSHARP_EXAMPLE", str(
            root / "csharp/RNet.Quickstart/bin/Release/net8.0/RNet.Quickstart.dll"))
        return subprocess.run([os.environ.get("DOTNET", "dotnet"), assembly, *args],
                              capture_output=True, text=True, timeout=30, check=False)

    def test_transports_with_actual_backpressure(self):
        for transport in ("tcp", "udp", "kcp"):
            for flags in ((), ("--one-slot",)):
                with self.subTest(transport=transport, flags=flags):
                    result = self.run_example(transport, *flags)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("received=2", result.stdout)
                    self.assertIn("both Stop/Dispose sequences completed", result.stdout)
                    self.assertIn("runtime=", result.stderr)
                    blocked = re.search(r"blocked=(\d+)", result.stdout)
                    self.assertIsNotNone(blocked)
                    if flags:
                        self.assertGreater(int(blocked.group(1)), 0)

    def test_denied_auth_is_not_a_timeout(self):
        for transport in ("tcp", "udp", "kcp"):
            with self.subTest(transport=transport):
                result = self.run_example(transport, "--deny")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertIn("code=-13", result.stderr)
                self.assertNotIn("deadline", result.stderr.lower())
                self.assertNotIn("cleanup", result.stderr.lower())

    def test_default_and_invalid_arguments(self):
        result = self.run_example()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("received=2", result.stdout)
        for args in (("unknown",), ("--one-slot",), ("tcp", "--bad"),
                     ("udp", "--deny", "--deny"), ("kcp", "--one-slot", "--one-slot")):
            with self.subTest(args=args):
                result = self.run_example(*args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertIn("usage:", result.stderr)
                self.assertNotIn("runtime=", result.stderr)


if __name__ == "__main__":
    unittest.main()
