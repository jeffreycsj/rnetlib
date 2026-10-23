"""Execute the real C++11 CLI, including failure exit codes and real queue pressure."""
from pathlib import Path
import re
import subprocess
import unittest


class CppGameExampleTests(unittest.TestCase):
    def run_example(self, *args):
        binary = Path(__file__).resolve().parents[1] / "target/rnet-cpp-game-quickstart"
        return subprocess.run([str(binary), *args], capture_output=True, text=True,
                              timeout=30, check=False)

    def test_all_transports_and_real_single_slot_backpressure(self):
        for transport in ("tcp", "udp", "kcp"):
            for flags in ((), ("--one-slot",)):
                with self.subTest(transport=transport, flags=flags):
                    result = self.run_example(transport, *flags)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("received=2", result.stdout)
                    self.assertIn("both runtimes closed", result.stdout)
                    if flags:
                        blocked = re.search(r"blocked=(\d+)", result.stdout)
                        self.assertIsNotNone(blocked, result.stdout)
                        self.assertGreater(int(blocked[1]), 0)

    def test_denied_authorization_retains_original_category(self):
        for transport in ("tcp", "udp", "kcp"):
            with self.subTest(transport=transport):
                result = self.run_example(transport, "--deny")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("example code=-13", result.stderr)
                self.assertNotIn("echo deadline", result.stderr)
                self.assertNotIn("cleanup", result.stderr)
                self.assertNotIn("game echo complete:", result.stdout)

    def test_wrong_server_key_never_reaches_application_authorization(self):
        for transport in ("tcp", "udp", "kcp"):
            with self.subTest(transport=transport):
                result = self.run_example(transport, "--wrong-key")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertNotIn("game_auth_requested", result.stderr)
                self.assertNotIn("game_session_ready", result.stderr)
                self.assertNotIn("game echo complete:", result.stdout)
                self.assertNotIn("echo deadline", result.stderr)
                self.assertNotIn("cleanup", result.stderr)
                # Two independent peers race: the server may observe handshake EOF
                # before the application polls the client's specific pin rejection.
                if "example code=-7 " in result.stderr:
                    self.assertIn("phase=handshake IoError: unexpected end of file", result.stderr)
                else:
                    self.assertRegex(result.stderr, r"example code=-(12|14|16)\b")

    def test_default_and_invalid_cli_arguments(self):
        result = self.run_example()
        self.assertEqual(result.returncode, 0, result.stderr)
        for args in (("quic",), ("tcp", "--unknown"), ("tcp", "--deny", "--deny"),
                     ("tcp", "--one-slot", "--one-slot")):
            with self.subTest(args=args):
                result = self.run_example(*args)
                self.assertEqual(result.returncode, 2)
                self.assertIn("usage:", result.stderr)


if __name__ == "__main__":
    unittest.main()
