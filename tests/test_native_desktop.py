"""Guard native verification against wrong-window and legacy-title false proofs."""
import itertools
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from native_desktop import NativeDesktop


class NativeWindowSelection(unittest.TestCase):
    def adapter(self):
        adapter = NativeDesktop("x11", {"DISPLAY": ":0"}, Path("/tmp"), Path("/tmp/owned-app"))
        adapter.alive = Mock()
        adapter.assert_focus = Mock()
        return adapter

    def test_missing_new_target_never_reuses_previous_window_or_activates_it(self):
        adapter = self.adapter()
        adapter.window = "previous-dashboard"
        adapter.run = Mock(return_value=subprocess.CompletedProcess([], 1, "", ""))
        with patch("native_desktop.time.monotonic", side_effect=itertools.count()), patch("native_desktop.time.sleep"):
            with self.assertRaisesRegex(RuntimeError, "No owned Rustleblower window"):
                adapter.attach(Mock(), 123, title="permission")
        self.assertFalse(any("windowactivate" in call.args[0] for call in adapter.run.call_args_list))

    def test_x11_unicode_title_uses_ewmh_name_with_exact_owned_pid(self):
        adapter = self.adapter()
        def run(command, **_):
            if command[1] == "search":
                self.assertIn("--pid", command)
                self.assertEqual(command[command.index("--pid") + 1], "123")
                if "--name" in command:  # The legacy STRING lookup misses Unicode.
                    return subprocess.CompletedProcess(command, 1, "", "")
                return subprocess.CompletedProcess(command, 0, "42\n43\n", "")
            if command[1] == "getwindowname":
                return subprocess.CompletedProcess(command, 0, "Rustleblower\n" if command[2] == "42" else "Rustleblower — connection permission\n", "")
            return subprocess.CompletedProcess(command, 0, "", "")
        adapter.run = Mock(side_effect=run)
        with patch("native_desktop.time.monotonic", side_effect=itertools.count()), patch("native_desktop.time.sleep"):
            adapter.attach(Mock(), 123, title="^Rustleblower — connection permission$")
        self.assertEqual(adapter.window, "43")
        adapter.run.assert_any_call(["xdotool", "windowactivate", "--sync", "43"])


if __name__ == "__main__":
    unittest.main()
