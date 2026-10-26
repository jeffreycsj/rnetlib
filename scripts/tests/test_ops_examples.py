"""Cross-check rule dependencies against the contract exercised by a real Rust runtime."""
import json
import os
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
OPS = ROOT / "examples" / "operations"


class OperationsExamplesTests(unittest.TestCase):
    def test_plain_make_keeps_the_existing_library_build_default(self):
        result = subprocess.run(["make", "-n"], cwd=ROOT, capture_output=True,
                                text=True, timeout=5, check=True)
        self.assertIn("scripts/generated_header.py --check", result.stdout)
        self.assertIn("cargo build -p rnet-ffi --release", result.stdout)
        self.assertNotIn("scripts/check-operations.sh", result.stdout)

    def test_rules_only_reference_runtime_contract_and_prometheus_up(self):
        selectors = (OPS / "metric-selectors.txt").read_text().splitlines()
        names = {line.split("{")[0] for line in selectors if line and not line.startswith("#")}
        rules = json.loads((OPS / "alerts.rules.json").read_text())["groups"][0]["rules"]
        self.assertGreaterEqual(len(rules), 10)
        for rule in rules:
            for name in re.findall(r"\brnet_[a-z0-9_]+", rule["expr"]):
                self.assertIn(name, names, rule["alert"])
            self.assertNotRegex(rule["expr"], r"histogram_quantile|\bavg\(")
            self.assertEqual(set(rule["labels"]), {"severity"})
            self.assertIn(rule["labels"]["severity"], {"warning", "critical"})
        self.assertEqual(len({rule["alert"] for rule in rules}), len(rules))

    def test_scrape_example_uses_operator_owned_local_endpoint(self):
        config = json.loads((OPS / "prometheus.example.json").read_text())
        job, = config["scrape_configs"]
        self.assertEqual(job["job_name"], "rnet-game")
        self.assertFalse(job["honor_labels"])
        self.assertEqual(job["static_configs"][0]["targets"], ["127.0.0.1:9464"])
        self.assertEqual(config["rule_files"], ["alerts.rules.json"])

    def test_every_shipped_alert_has_a_positive_official_evaluator_case(self):
        rules = json.loads((OPS / "alerts.rules.json").read_text())["groups"][0]["rules"]
        covered = set()
        for path in OPS.glob("*.test.json"):
            for scenario in json.loads(path.read_text())["tests"]:
                covered.update(case["alertname"] for case in scenario["alert_rule_test"]
                               if case["exp_alerts"])
        self.assertEqual({rule["alert"] for rule in rules}, covered)

    def test_missing_validator_fails_instead_of_skipping_the_gate(self):
        result = subprocess.run(["bash", str(ROOT / "scripts/check-operations.sh")],
                                env=dict(os.environ, PROMTOOL="/nonexistent-rnet-validator/promtool"),
                                capture_output=True, text=True, timeout=5, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("promtool is required", result.stderr)


if __name__ == "__main__":
    unittest.main()
