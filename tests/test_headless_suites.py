"""Keep parallel GPU coverage complete and distinguish compilation from input time."""
import ast
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import re
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("headless_suites", ROOT / "scripts/headless_suites.py")
suites = importlib.util.module_from_spec(spec)
spec.loader.exec_module(suites)


class BatchCoverage(unittest.TestCase):
    def test_workflow_runs_every_batch_and_every_journey_once(self):
        suites.validate_shards()
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        matrix = re.search(r"^\s+shard: (\[[^\n]+\])$", workflow, re.MULTILINE)
        self.assertIsNotNone(matrix, "The workflow must enumerate the GPU batches")
        self.assertEqual(ast.literal_eval(matrix[1]), list(range(len(suites.SHARDS))))
        self.assertEqual(len(set(suites.FEATURE_TESTS.values())), len(suites.FEATURE_TESTS))

    def test_missing_duplicate_unknown_and_empty_batches_are_refused(self):
        batches = [list(batch) for batch in suites.SHARDS]
        missing = [list(batch) for batch in batches]
        missing[0].pop()
        duplicate = [list(batch) for batch in batches]
        duplicate[1].append(duplicate[0][0])
        unknown = [list(batch) for batch in batches]
        unknown[0][0] = "unknown-feature"
        for invalid in [missing, duplicate, unknown, [*batches, []], []]:
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                suites.validate_shards(invalid)

    def test_batch_cli_produces_only_its_assigned_journeys(self):
        for index, batch in enumerate(suites.SHARDS):
            output = io.StringIO()
            with patch("sys.argv", ["headless_suites.py", "--shard", str(index)]), redirect_stdout(output):
                suites.main()
            self.assertEqual(output.getvalue().split(), list(batch))


class LogTimings(unittest.TestCase):
    def test_cold_build_and_journey_are_distinct(self):
        log = "Finished `test` profile [unoptimized] target(s) in 1m 50s\n" \
              "test result: ok. 1 passed; 0 failed; finished in 30.68s\n"
        self.assertEqual(suites.log_timings(log), {"build_seconds": 110.0, "test_seconds": 30.68})

    def test_cached_ansi_build_and_failed_journey_keep_real_durations(self):
        log = "\x1b[1m\x1b[92m    Finished\x1b[0m `test` profile target(s) in 0.35s\n" \
              "test result: FAILED. 0 passed; 1 failed; finished in 2.05s\n"
        self.assertEqual(suites.log_timings(log), {"build_seconds": 0.35, "test_seconds": 2.05})

    def test_compile_failure_does_not_claim_zero_runtime(self):
        self.assertEqual(suites.log_timings("error: compilation failed\n"),
                         {"build_seconds": None, "test_seconds": None})

    def test_hours_and_build_only_output_are_supported(self):
        self.assertEqual(suites.log_timings("Finished `test` profile target(s) in 1h 2m 3.5s\n"),
                         {"build_seconds": 3723.5, "test_seconds": None})


if __name__ == "__main__":
    unittest.main()
