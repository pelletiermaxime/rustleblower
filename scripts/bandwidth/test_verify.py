"""Regression coverage for failures hidden by the original generic timeout."""
import copy
import io
import os
import unittest

from verify import Lines, ReadTimeout, check_attribution, observe


def sample(sent=0, received=0, timestamp=20):
    return {"version": 1, "started_ms": 1, "sample_ms": timestamp,
            "rows": [{"pid": 10, "uid": 1000, "start_ns": 1, "inode": 1,
                      "executable_hex": "61", "sent": sent, "received": received}],
            "lost_operations": 0, "truncated": False,
            "diagnostics": {"entries": 4, "returns": 4, "accounted_calls": 4}}


class Samples:
    def __init__(self, *samples):
        self.samples = iter(samples)

    def read(self, timeout):
        try:
            return next(self.samples)
        except StopIteration:
            raise ReadTimeout("Timed out waiting for collector output") from None


class DiagnosticsTest(unittest.TestCase):
    def setUp(self):
        self.result = {"workload": {"pid": 10, "uid": 1000}, "started_ms": 1}
        self.target = {"sent": 100, "received": 100}

    def test_continuous_zero_snapshots_report_actual_bytes_and_diagnostics(self):
        reader = Samples(sample(), sample(timestamp=21))
        with self.assertRaisesRegex(RuntimeError, r"Byte totals did not reach.*observed=.*snapshots=2"):
            observe(reader, self.target, 10, False, self.result)
        self.assertEqual(self.result["observed_totals"], {"sent": 0, "received": 0})
        self.assertEqual(self.result["last_snapshot"]["diagnostics"]["entries"], 4)
        self.assertEqual(self.result["expected_totals"], self.target)

    def test_missing_collector_output_is_distinct_from_zero_counters(self):
        with self.assertRaisesRegex(RuntimeError, r"observed=None, snapshots=0"):
            observe(Samples(), self.target, 10, False, self.result)

    def test_only_snapshot_after_workload_completion_can_pass(self):
        old = sample(100, 100, timestamp=9)
        fresh = sample(100, 100, timestamp=11)
        actual = observe(Samples(old, fresh), self.target, 10, False, self.result)
        self.assertEqual(actual["sample_ms"], 11)
        self.assertEqual(self.result["case_snapshots"], 2)

    def test_partial_counter_progress_remains_in_failure_report(self):
        with self.assertRaisesRegex(RuntimeError, r"'sent': 100, 'received': 64"):
            observe(Samples(sample(100, 64)), self.target, 10, False, self.result)

    def test_overcount_fails_immediately(self):
        with self.assertRaisesRegex(AssertionError, "Counters exceeded expected bytes"):
            observe(Samples(sample(101, 100)), self.target, 10, False, self.result)
        self.assertEqual(self.result["last_snapshot"]["rows"][0]["sent"], 101)

    def test_loss_or_truncation_fails_without_waiting(self):
        for field, value in (("lost_operations", 1), ("truncated", True)):
            with self.subTest(field=field):
                data = sample(100, 100)
                data[field] = value
                with self.assertRaises(AssertionError):
                    observe(Samples(data), self.target, 10, False, copy.deepcopy(self.result))

    def test_dead_process_requires_captured_path_and_retains_totals(self):
        data = sample(100, 100)
        data["rows"][0]["executable_source"] = "kernel"
        self.assertEqual(observe(Samples(data), self.target, 10, True, self.result), data)
        check_attribution(data, True)

    def test_live_unknown_path_still_fails_with_reason(self):
        data = sample(100, 100)
        data["rows"][0].update(executable_hex=None, executable_unavailable_reason="filesystem-device-mismatch")
        with self.assertRaisesRegex(AssertionError, "filesystem-device-mismatch"):
            check_attribution(data, False)

    def test_exited_process_cannot_depend_on_a_current_proc_path(self):
        with self.assertRaisesRegex(AssertionError, "kernel metadata capture"):
            check_attribution(sample(100, 100), True)

    def test_reader_names_which_process_stopped_responding(self):
        fd, write_fd = os.pipe()
        with io.FileIO(fd, "rb") as stream:
            reader = Lines(stream, "worker")
            try:
                with self.assertRaisesRegex(ReadTimeout, "worker output"):
                    reader.read(timeout=0.01)
            finally:
                reader.close()
                os.close(write_fd)


if __name__ == "__main__":
    unittest.main()
