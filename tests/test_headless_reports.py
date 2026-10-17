"""Require honest Kit interaction evidence rather than stale screenshot reports."""
import importlib.util
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
spec = importlib.util.spec_from_file_location("verify_headless", ROOT / "scripts/verify-headless.py")
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


class KitEvidence(unittest.TestCase):
    def setUp(self):
        self.owned = tempfile.TemporaryDirectory()
        self.addCleanup(self.owned.cleanup)
        self.evidence = Path(self.owned.name)
        (self.evidence / "layout.json").write_text(json.dumps({"kind": "kit-layout-checkpoint"}))
        self.flow = {"status": "passed", "framework": "gpui-kit", "real_gpu": False,
                     "scoped_socket_removed": True, "checkpoints": ["layout.json"]}

    def test_kit_report_with_json_checkpoint_is_accepted(self):
        verifier.validate_flow(self.flow, self.evidence)

    def test_gpu_claim_missing_framework_and_unclean_socket_are_refused(self):
        for key, value in (("real_gpu", True), ("framework", ""), ("scoped_socket_removed", False)):
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                verifier.validate_flow(dict(self.flow, **{key: value}), self.evidence)

    def test_images_missing_checkpoints_and_malformed_json_are_refused(self):
        for checkpoints in ([], ["missing.json"], ["old.png"]):
            with self.subTest(checkpoints=checkpoints), self.assertRaises(RuntimeError):
                verifier.validate_flow(dict(self.flow, checkpoints=checkpoints), self.evidence)
        (self.evidence / "layout.json").write_text("invalid")
        with self.assertRaises(json.JSONDecodeError):
            verifier.validate_flow(self.flow, self.evidence)

    def test_complete_suite_reports_a_single_failed_journey_as_failure(self):
        destination = self.evidence / "suite"

        def run(feature, evidence, timeout):
            return {"feature": feature, "evidence": str(evidence),
                    "status": "failed" if feature == "settings" else "passed"}

        with patch.object(verifier, "verify_feature", side_effect=run) as verify, \
             patch("sys.argv", ["verify-headless.py", "--all", "--evidence", str(destination)]), \
             redirect_stdout(io.StringIO()):
            self.assertEqual(verifier.main(), 1)
        report = json.loads((destination / "result.json").read_text())
        self.assertEqual(verify.call_count, 42)
        self.assertEqual(report["passed"], 41)
        self.assertEqual(report["status"], "failed")
        self.assertIs(report["real_gpu"], False)


if __name__ == "__main__":
    unittest.main()
