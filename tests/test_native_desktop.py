"""Guard native verification against wrong-window and legacy-title false proofs."""
import itertools
from pathlib import Path
import subprocess
import sys
import tempfile
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


class NativeUrlInput(unittest.TestCase):
    def test_canadian_french_slashes_use_actual_layout_keys_without_changing_layout(self):
        with tempfile.TemporaryDirectory() as directory:
            adapter=NativeDesktop("wayland",{},Path(directory),Path("/tmp/owned-app"))
            adapter.assert_focus=Mock()
            adapter.type=Mock()
            def run(command):
                output='0' if command[-1].endswith('getLayout') else '[Argument: a(sss) {[Argument: (sss) "ca", "", "French (Canada)"]}]'
                return subprocess.CompletedProcess(command,0,output,'')
            adapter.run=Mock(side_effect=run)
            adapter.type_url("http://127.0.0.1:9/owned-native-list")
            self.assertEqual([call.args[0] for call in adapter.type.call_args_list],["http:","127.0.0.1:9","owned-native-list"])
            keys=[call.args[0] for call in adapter.run.call_args_list if call.args[0][0]=='ydotool']
            self.assertEqual(keys,[["ydotool","key","42:1","4:1","4:0","42:0"]]*3)
            self.assertFalse(any('setLayout' in str(call) for call in adapter.run.call_args_list))

    def test_unknown_layout_refuses_before_typing(self):
        adapter=NativeDesktop("wayland",{},Path("/tmp"),Path("/tmp/owned-app"))
        adapter.assert_focus=Mock()
        adapter.type=Mock()
        adapter.run=Mock(side_effect=[subprocess.CompletedProcess([],0,'0',''),subprocess.CompletedProcess([],0,'[Argument: (sss) "fr", "", "French"]','')])
        with self.assertRaisesRegex(RuntimeError,"not verified for layout"):
            adapter.type_url("http://127.0.0.1:9/owned-native-list")
        adapter.type.assert_not_called()

    def test_x11_url_uses_existing_text_input(self):
        adapter=NativeDesktop("x11",{},Path("/tmp"),Path("/tmp/owned-app"))
        adapter.type=Mock()
        adapter.type_url("http://127.0.0.1:9/owned-native-list")
        adapter.type.assert_called_once_with("http://127.0.0.1:9/owned-native-list")


if __name__ == "__main__":
    unittest.main()
