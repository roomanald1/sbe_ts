import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("learning_coverage.py")


def profile(symbol, evidence, threshold=10, max_age=86400):
    return {
        "symbol": symbol,
        "strategy": "live-momentum",
        "parameters": {
            "scout_minimum_walk_forward_samples": threshold,
            "scout_maximum_evidence_age_seconds": max_age,
        },
        "walk_forward_evidence": evidence,
    }


def evidence(**overrides):
    value = {
        "sampleCount": 50,
        "effectiveSampleSize": 30.0,
        "lastSampleTimestampMs": 1_000_000,
        "historyCoverage": "complete",
        "costCoverage": "complete",
        "searchFamilyCoverage": "complete",
        "finalHoldoutStatus": "validated",
        "expectancyCiLowPercent": 0.1,
        "evidenceScope": "exact_profile_unconditioned",
    }
    value.update(overrides)
    return value


class LearningCoverageTests(unittest.TestCase):
    def run_report(self, body):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_path = root / "input.json"
            output_path = root / "coverage.json"
            input_path.write_text(json.dumps({"body": body}))
            run = subprocess.run(
                [sys.executable, str(SCRIPT), "--input", str(input_path),
                 "--output", str(output_path), "--limit", "100"],
                capture_output=True, text=True, check=True,
            )
            payload = json.loads(output_path.read_text())
            return payload["summary"]

    def test_missing_zero_stale_incomplete_and_ready_remain_distinct(self):
        summary = self.run_report({
            "refreshed_at": 1_000_000,
            "strategy_profiles": [
                profile("MISSING-USDT", None),
                profile("ZERO-USDT", evidence(sampleCount=0, effectiveSampleSize=0.0)),
                profile("STALE-USDT", evidence(lastSampleTimestampMs=1, historyCoverage="partial"), max_age=10),
                profile("INCOMPLETE-USDT", evidence(costCoverage="unknown", finalHoldoutStatus="not_validated")),
                profile("READY-USDT", evidence()),
            ],
            "contexts": [{
                "context": "long:trend:BTC-USDT:live-momentum",
                "learning_samples": 0,
                "scout_learning_samples": 0,
            }],
        })
        self.assertEqual(summary["profiles"], 5)
        self.assertEqual(summary["contexts"], 1)
        self.assertEqual(summary["ready"], 1)
        self.assertEqual(summary["missing_evidence"], 2)
        self.assertEqual(summary["zero_effective_sample_rows"], 1)
        self.assertGreaterEqual(summary["stale_rows"], 1)
        self.assertEqual(summary["unknown_effective_sample_rows"], 2)
        self.assertEqual(summary["profiles_with_gaps"], 4)
        self.assertEqual(summary["contexts_with_gaps"], 1)


if __name__ == "__main__":
    unittest.main()
