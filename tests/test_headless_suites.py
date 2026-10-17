"""Keep Kit coverage complete and distinguish compilation from input time."""
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("headless_suites", ROOT / "scripts/headless_suites.py")
suites = importlib.util.module_from_spec(spec)
spec.loader.exec_module(suites)


class JourneyCoverage(unittest.TestCase):
    def test_workflow_runs_the_complete_suite_on_one_runner(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text().split("  headless:\n", 1)[1]
        self.assertNotIn("strategy:", workflow)
        self.assertNotIn("matrix.", workflow)
        self.assertEqual(workflow.count("--features headless-tests --no-run"), 1)
        self.assertEqual(workflow.count("scripts/verify-headless.py --all"), 1)
        self.assertNotIn("scripts/verify-headless.py --feature", workflow)
        self.assertEqual(len(set(suites.FEATURE_TESTS.values())), len(suites.FEATURE_TESTS))

    def test_every_registered_journey_uses_the_kit_runner(self):
        registered = set()
        for target in suites.FEATURE_TESTS.values():
            module, test = target.split("::")
            source = (ROOT / "src/app/kit_tests" / f"{module}.rs").read_text()
            self.assertRegex(source, rf"#\[gpui_kit::test\]\s+(?:async\s+)?fn {test}\(")
            registered.add(module)
        actual = {path.stem for path in (ROOT / "src/app/kit_tests").glob("*.rs")
                  if "#[gpui_kit::test]" in path.read_text()}
        self.assertEqual(registered, actual)
        self.assertEqual(len(registered), 42)

    def test_inventory_cli_lists_every_journey_once(self):
        output = io.StringIO()
        with patch("sys.argv", ["headless_suites.py"]), redirect_stdout(output):
            suites.main()
        self.assertEqual(output.getvalue().split(), list(suites.FEATURE_TESTS))


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
