"""Guard the development launcher's protocol and isolated application arguments."""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest


class HotReloadLauncherTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copyfile(Path(__file__).resolve().parents[1] / "scripts/dev-hot-reload.sh",
                        scripts / "dev-hot-reload.sh")
        (scripts / "prepare_dependencies.py").write_text(
            "from pathlib import Path\nPath('prepared').touch()\n")
        self.dx = self.root / "dx"
        self.dx.write_text("""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
if sys.argv[1:] == ['--version']:
    print(os.environ.get('FAKE_DX_VERSION', 'dioxus 0.7.10 (fixture)'))
else:
    Path('launch.json').write_text(json.dumps({'args': sys.argv[1:], 'log': os.environ['RUST_LOG']}))
""")
        self.dx.chmod(0o700)
        self.env = {**os.environ, "RUSTLEBLOWER_DX": str(self.dx)}
        self.env.pop("RUST_LOG", None)
        self.env.pop("RUSTLEBLOWER_DEV_DIR", None)

    def launch(self, *args):
        return subprocess.run(["bash", str(self.root / "scripts/dev-hot-reload.sh"), *args],
                              cwd=self.root, env=self.env, capture_output=True, text=True)

    def test_isolated_paths_and_matching_protocol(self):
        directory = self.root / "state with spaces and 'quotes'"
        self.env["RUSTLEBLOWER_DEV_DIR"] = str(directory)
        result = self.launch("--port", "8081")
        self.assertEqual(result.returncode, 0, result.stderr)
        launch = json.loads((self.root / "launch.json").read_text())
        args = launch["args"]
        self.assertTrue((self.root / "prepared").exists())
        self.assertIn("--hot-patch", args)
        self.assertIn("--cargo-args=--locked", args)
        self.assertIn("rustleblower-dev", args)
        self.assertEqual(args[-2:], ["--port", "8081"])
        app_args = next(arg.removeprefix("--args=") for arg in args if arg.startswith("--args="))
        self.assertEqual(shlex.split(app_args), ["--socket", str(directory / "ui.sock"),
                                               "--database", str(directory / "history.sqlite3")])
        self.assertEqual(launch["log"], "warn,gpui=info")

    def test_explicit_app_arguments_replace_defaults(self):
        for supplied in [("--args=--headless",), ("--args", "--headless")]:
            with self.subTest(supplied=supplied):
                result = self.launch(*supplied)
                self.assertEqual(result.returncode, 0, result.stderr)
                args = json.loads((self.root / "launch.json").read_text())["args"]
                self.assertEqual(args[-len(supplied):], list(supplied))
                self.assertEqual(sum(arg == "--args" or arg.startswith("--args=") for arg in args), 1)

    def test_wrong_protocol_stops_before_dependency_preparation(self):
        self.env["FAKE_DX_VERSION"] = "dioxus 0.7.9 (fixture)"
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Dioxus CLI 0.7.10", result.stderr)
        self.assertFalse((self.root / "prepared").exists())
        self.assertFalse((self.root / "launch.json").exists())

    def test_missing_cli_stops_before_dependency_preparation(self):
        self.env["RUSTLEBLOWER_DX"] = str(self.root / "missing-dx")
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cargo install dioxus-cli --version 0.7.10 --locked", result.stderr)
        self.assertFalse((self.root / "prepared").exists())


if __name__ == "__main__":
    unittest.main()
