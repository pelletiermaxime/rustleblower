#!/usr/bin/env python3
"""Decode and launch only an owned Rust startup-entry fixture through GIO."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--fixtures", type=Path, required=True)
    p.add_argument("--evidence", type=Path, required=True)
    args = p.parse_args()
    source = args.fixtures.resolve()
    evidence = args.evidence.resolve()
    evidence.mkdir(mode=0o700, exist_ok=False)
    fixture = json.loads((source / "fixture.json").read_text())
    entry, executable, output = (Path(fixture[k]).resolve() for k in ("entry", "executable", "argv"))
    assert all(path.is_relative_to(source) for path in (entry, executable, output)), "Fixture path escaped its owned root"
    assert executable.read_text() == "#!/usr/bin/python3\nimport json,sys\nfrom pathlib import Path\nPath(__file__).with_name('argv.json').write_text(json.dumps(sys.argv[1:]))\n", "Unexpected fixture executable"
    assert not output.exists(), "Use a fresh owned fixture"
    arguments = fixture["expected_arguments"]
    assert len(arguments) == 8 and arguments[::2] == ["--socket", "--database", "--prompt-timeout", "--timeout-action"]
    assert all(isinstance(a, str) and len(a.encode()) <= 4096 and not any(ord(c) < 32 for c in a) for a in arguments)
    assert all(Path(arguments[i]).is_absolute() and Path(arguments[i]).resolve().is_relative_to(source) for i in (1, 3))
    assert arguments[5].isdigit() and 1 <= int(arguments[5]) <= 110 and arguments[7] in ("allow", "deny", "reject")

    def quote(value):
        text = '"' + ''.join("%%" if c == "%" else "\\" + c if c in '\\"$`' else c for c in value) + '"'
        return text.replace("\\", "\\\\")

    command = ([quote("/usr/bin/env"), "--", quote(str(executable))] if "%" in str(executable) else [quote(str(executable))])
    command.extend(quote(a) for a in arguments)
    # Guard execution independently of desktop decoding: reject extra keys,
    # groups or a changed Exec before passing the owned entry to GIO.
    expected = {"Type": "Application", "Version": "1.0", "Name": "Rustleblower", "Comment": "OpenSnitch firewall interface", "Exec": " ".join(command), "Icon": "dev.rustleblower.Rustleblower", "Terminal": "false", "Hidden": "false", "X-Rustleblower-Managed": "true"}
    lines = entry.read_text().splitlines()
    assert len(entry.read_bytes()) <= 65536 and lines[0] == "[Desktop Entry]"
    pairs = [line.split("=", 1) for line in lines[1:]]
    assert all(len(p) == 2 for p in pairs) and len(pairs) == len(expected) and dict(pairs) == expected, "Unexpected fixture entry; refusing execution"
    env = os.environ.copy()
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "DBUS_STARTER_BUS_TYPE", "DBUS_STARTER_ADDRESS"):
        env.pop(key, None)
    env["XDG_CONFIG_HOME"] = str(source / "xdg")
    env["XDG_CONFIG_DIRS"] = str(source / "empty-system")
    env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=" + str(source / "no-session-bus")
    result = {"status": "failed", "entry": str(entry), "executable": str(executable)}
    try:
        validation = subprocess.run(["desktop-file-validate", str(entry)], env=env, capture_output=True, text=True, timeout=10)
        (evidence / "validation.log").write_text(validation.stdout + validation.stderr)
        assert validation.returncode == 0, "Desktop entry validation failed"
        launched = subprocess.run(["gio", "launch", str(entry)], env=env, capture_output=True, text=True, timeout=10)
        (evidence / "gio.log").write_text(launched.stdout + launched.stderr)
        assert launched.returncode == 0, "GIO launch failed"
        deadline = time.monotonic() + 5
        while not output.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        actual = json.loads(output.read_text())
        assert actual == fixture["expected_arguments"], "Desktop Exec changed argument boundaries or literal characters"
        result.update(status="passed", exact_arguments=True, expected_arguments=fixture["expected_arguments"], actual_arguments=actual, no_display_or_session_bus=True, proof_boundary="Real desktop-file validation and GIO argument decoding/launch of a fixed owned recording fixture. No Rustleblower/system-daemon launch, real login scheduler or compositor proof.")
    except Exception as e:
        result["error"] = str(e)
    (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"status": result["status"], "evidence": str(evidence)}))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
