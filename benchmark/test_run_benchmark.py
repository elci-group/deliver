import csv
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from run_benchmark import cost_usd, protected_digests

BENCHMARK = Path(__file__).resolve().parent


class BenchmarkRunnerTests(unittest.TestCase):
    def test_cost_uses_cached_and_uncached_rates(self):
        detail = {"input_tokens": 1000, "cached_input_tokens": 600,
                  "output_tokens": 200, "reasoning_output_tokens": 20}
        self.assertEqual(cost_usd(detail, {"input": 2.0, "cached_input": 0.2, "output": 10.0}), 0.00292)
        self.assertIsNone(cost_usd(detail, {"input": 2.0, "cached_input": None, "output": 10.0}))

    def test_protected_digests_follow_the_fixture_convention(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Path(temporary)
            for name in ("README.md", "deliver.toml", "test_task.py", "task.py", "notes.txt"):
                (fixture / name).write_text(name)
            digests = protected_digests(fixture)
        self.assertEqual(sorted(digests), ["README.md", "deliver.toml", "test_task.py"])


    def test_runs_balanced_pairs_and_reports_usage(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "results"
            command = f"{sys.executable} {BENCHMARK / 'tests' / 'fake_agent.py'} --cwd {{workdir}} {{prompt}}"
            completed = subprocess.run(
                [sys.executable, str(BENCHMARK / "run_benchmark.py"), "--model", "test-model",
                 "--repetitions", "2", "--runs-dir", str(output), "--agent-cmd", command],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            summary = json.loads((output / "summary.json").read_text())
            self.assertTrue(summary["valid_comparison"])
            self.assertEqual(summary["conditions"]["control"]["agent_completion_rate"], 1)
            self.assertEqual(summary["conditions"]["control"]["acceptance_rate"], 1)
            self.assertEqual(summary["conditions"]["deliver"]["mean_tokens_reported"], 47)
            with (output / "trials.csv").open() as handle:
                rows = list(csv.DictReader(handle))
            self.assertEqual(len(rows), 4)
            self.assertTrue(all(row["acceptance_pass"] == "True" for row in rows))

    def test_marks_nonzero_agent_exits_as_an_invalid_comparison(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "results"
            command = f"{sys.executable} {BENCHMARK / 'tests' / 'fake_agent.py'} --exit 3 --cwd {{workdir}} {{prompt}}"
            completed = subprocess.run(
                [sys.executable, str(BENCHMARK / "run_benchmark.py"), "--model", "test-model",
                 "--repetitions", "2", "--runs-dir", str(output), "--agent-cmd", command],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            summary = json.loads((output / "summary.json").read_text())
            self.assertFalse(summary["valid_comparison"])
            self.assertIn("control", summary["invalid_reason"])
            self.assertEqual(summary["conditions"]["control"]["acceptance_rate_when_completed"], None)


if __name__ == "__main__":
    unittest.main()
