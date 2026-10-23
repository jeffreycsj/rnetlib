"""End-to-end C example checks; invoked by make c-test after compiling binaries."""
from pathlib import Path
import os
import subprocess
import unittest


class CGameExampleTests(unittest.TestCase):
    def run_example(self, *args, binary="rnet-c-game-echo", always_block=False):
        root = Path(__file__).resolve().parents[1]
        environment = os.environ.copy()
        environment.pop("RNET_EXAMPLE_ALWAYS_BLOCK", None)
        if always_block:
            environment["RNET_EXAMPLE_ALWAYS_BLOCK"] = "1"
        return subprocess.run([str(root / "target" / binary), *args],
                              capture_output=True, text=True, timeout=30, check=False, env=environment)

    def test_all_transports_echo_binary_and_empty_payloads(self):
        for transport in ("tcp", "udp", "kcp"):
            with self.subTest(transport=transport):
                result = self.run_example(transport)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"{transport} game echo complete", result.stdout)
                self.assertIn("runtimes destroyed", result.stdout)

    def test_backpressure_preserves_both_peers_payloads(self):
        for transport in ("tcp", "udp", "kcp"):
            with self.subTest(transport=transport):
                result = self.run_example(transport, binary="rnet-c-game-backpressure")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr.count("injected send backpressure"), 4)
                self.assertIn("game echo complete", result.stdout)

    def test_authorization_denial_is_not_a_timeout_or_success(self):
        for transport in ("tcp", "udp", "kcp"):
            with self.subTest(transport=transport):
                result = self.run_example(transport, "--deny")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertRegex(result.stderr, r"(?:client|server) code=-13 connection failed")
                self.assertNotIn("echo deadline", result.stderr)
                self.assertNotIn("shutdown code=", result.stderr)
                self.assertNotIn("game echo complete", result.stdout)

    def test_cli_rejects_unknown_transport_or_option(self):
        for args in ((), ("quic",), ("tcp", "--unknown")):
            with self.subTest(args=args):
                result = self.run_example(*args)
                self.assertEqual(result.returncode, 2)
                self.assertIn("usage:", result.stderr)

    def test_permanent_backpressure_times_out_and_cleans_up(self):
        result = self.run_example("tcp", binary="rnet-c-game-backpressure", always_block=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("client code=-5 echo deadline exceeded", result.stderr)
        self.assertNotIn("shutdown code=", result.stderr)
        self.assertNotIn("game echo complete", result.stdout)


if __name__ == "__main__":
    unittest.main()
