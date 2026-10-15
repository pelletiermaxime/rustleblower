#!/usr/bin/env python3
"""Owned foreground verification using FerryDock's KDE/Wayland or explicit X11 adapter."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import time
import uuid
from native_desktop import NativeDesktop, desktop_environment, stop_owned_process

ROOT = Path(__file__).resolve().parents[1]

def placement_geometry(adapter, position):
    """Read actual owned X11 bounds and independent RandR/EWMH expectations."""
    adapter.assert_focus()
    values = dict(line.split("=", 1) for line in adapter.run(["xdotool", "getwindowgeometry", "--shell", adapter.window]).stdout.splitlines() if "=" in line)
    client = {key.lower(): int(values[key]) for key in ("X", "Y", "WIDTH", "HEIGHT")}
    extents_text = adapter.run(["xprop", "-id", adapter.window, "_NET_FRAME_EXTENTS"]).stdout
    extents = list(map(int, re.findall(r"\d+", extents_text.split("=", 1)[1]))) if "=" in extents_text else [0, 0, 0, 0]
    assert len(extents) == 4, "Invalid native frame extents"
    left, right, top, bottom = extents
    actual = {"x": client["x"] - left, "y": client["y"] - top, "width": client["width"] + left + right, "height": client["height"] + top + bottom}
    monitors = adapter.run(["xrandr", "--listmonitors"]).stdout.splitlines()[1:33]
    primary = next((line for line in monitors if "*" in line), monitors[0])
    match = re.search(r"(\d+)/\d+x(\d+)/\d+([+-]\d+)([+-]\d+)", primary)
    assert match, "Cannot identify primary monitor geometry"
    width, height, x, y = map(int, match.groups())
    props = adapter.run(["xprop", "-root", "_NET_WORKAREA", "_NET_CURRENT_DESKTOP"]).stdout
    desktop_line = next((line for line in props.splitlines() if line.startswith("_NET_CURRENT_DESKTOP") and "=" in line), "")
    desktop = int(desktop_line.split("=", 1)[1].strip()) if desktop_line else 0
    work_line = next((line for line in props.splitlines() if line.startswith("_NET_WORKAREA") and "=" in line), "")
    work = list(map(int, re.findall(r"-?\d+", work_line.split("=", 1)[1]))) if work_line else []
    if len(work) >= desktop * 4 + 4:
        wx, wy, ww, wh = work[desktop * 4:desktop * 4 + 4]
        right, bottom = min(x + width, wx + ww), min(y + height, wy + wh)
        x, y = max(x, wx), max(y, wy)
        width, height = right - x, bottom - y
    scale = client["width"] / 650
    margin = 16 * scale
    expected_x = x + (width - actual["width"]) / 2 if position == "center" else x + margin if position.endswith("left") else x + width - margin - actual["width"]
    expected_y = y + (height - actual["height"]) / 2 if position == "center" else y + margin if position.startswith("top") else y + height - margin - actual["height"]
    delta = {"x": actual["x"] - expected_x, "y": actual["y"] - expected_y}
    tolerance = max(3, 2 * scale)
    return {"actual": actual, "client": client, "frame_extents": extents, "work_area": {"x": x, "y": y, "width": width, "height": height}, "expected_origin": {"x": expected_x, "y": expected_y}, "delta": delta, "tolerance": tolerance, "passed": max(abs(delta["x"]), abs(delta["y"])) <= tolerance}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["doctor", "run"])
    parser.add_argument("--backend", choices=["auto", "wayland", "x11"], default="auto")
    parser.add_argument("--display", default=os.environ.get("DISPLAY"))
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/rustleblower")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--decision", choices=["deny", "allow"], default="deny")
    parser.add_argument("--feature", choices=["history", "history-tools", "statistics", "nodes", "daemon-preferences", "prompts", "placement", "countdown", "checksums"], default="history")
    parser.add_argument("--position", choices=["center", "top-right", "bottom-right", "top-left", "bottom-left"], default="bottom-right")
    args = parser.parse_args()
    backend, env = desktop_environment(args.backend, args.display, os.environ)
    binary = args.binary.resolve()
    if args.command == "doctor":
        adapter = NativeDesktop(backend, env, Path(tempfile.gettempdir()), binary)
        result = adapter.doctor()
        result["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        print(json.dumps(result, indent=2))
        return 0
    evidence = (args.evidence or ROOT / ".verification/native" /
                (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8])).resolve()
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    result = {"status": "failed", "feature": args.feature, "backend": backend, "evidence": str(evidence)}
    app = fixture = None
    adapter = NativeDesktop(backend, env, evidence, binary)
    scratch = tempfile.TemporaryDirectory(prefix="rustleblower-verify-")
    socket = Path(scratch.name) / "ui.sock"
    lock = open(f"/tmp/ferrydock-native-verification-{os.getuid()}.lock", "a")
    app_log = (evidence / "application.log").open("w")
    fixture_log = (evidence / "fixture.log").open("w")
    actions = []
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        doctor = adapter.doctor()
        (evidence / "doctor.json").write_text(json.dumps(doctor, indent=2) + "\n")
        adapter.start_input()
        if args.feature == "placement":
            assert backend == "x11", "Placement geometry proof requires explicitly selected --backend x11; Wayland controls placement"
            (evidence / "history.sqlite3.preferences.json").write_text(json.dumps({"version": 3, "duration": "once", "target": "connection", "popups": True, "priority": False, "logging": True, "position": args.position}) + "\n")
        # Cargo can replace target/debug/rustleblower while another check builds
        # a headless binary. Own a snapshot so a run always drives one executable.
        snapshot = Path(scratch.name) / "rustleblower"
        shutil.copy2(binary, snapshot)
        adapter.binary = snapshot
        app = subprocess.Popen([str(snapshot), "--socket", str(socket), "--database", str(evidence / "history.sqlite3"),
                                "--prompt-timeout", "5" if args.feature == "countdown" else "30"],
                               env=env, stdout=app_log, stderr=app_log)
        adapter.attach(app, app.pid)
        (evidence / "instance.json").write_text(json.dumps({"pid": app.pid, "binary": str(snapshot), "source_binary": str(binary), "socket": str(socket),
            "window": adapter.window, "binary_sha256": hashlib.sha256(snapshot.read_bytes()).hexdigest()}, indent=2) + "\n")
        fixture = subprocess.Popen(["python", str(ROOT / "scripts/fake-daemon.py"), "--socket", str(socket),
            "--seconds", "30" if args.feature == "daemon-preferences" else "5", "--prompts", "1" if args.feature in ("prompts", "placement", "countdown", "checksums") else "0",
            "--checksum-rules", "1" if args.feature == "checksums" else "0", "--report", str(evidence / "fixture-report.json")] + (["--allow-node-config"] if args.feature == "daemon-preferences" else []),
            env=env, stdout=fixture_log, stderr=fixture_log)
        if args.feature in ("prompts", "placement", "countdown", "checksums"):
            adapter.attach(app, app.pid, title="^Rustleblower — connection permission$")
            adapter.capture(evidence / "01-prompt.png")
            if args.feature == "checksums":
                adapter.keys("Tab")
                time.sleep(0.2)
                adapter.capture(evidence / "02-checksum-link-focus.png")
                adapter.keys("space")
                adapter.attach(app, app.pid, title="^Rustleblower — checksum review$")
                time.sleep(0.3)
                adapter.capture(evidence / "03-checksum-review.png")
                ocr = adapter.run(["tesseract", str(evidence / "03-checksum-review.png"), "stdout"]).stdout
                assert "Saved:" in ocr and "Reported:" in ocr and "checksum" in ocr.lower(), "Native expected/reported checksum review missing"
                actions.append({"input": "Tab; Space on permission warning", "expected": "Separate owned checksum review displays saved/reported values"})
                adapter.attach(app, app.pid, title="^Rustleblower — connection permission$")
            if args.feature == "placement":
                geometry = placement_geometry(adapter, args.position)
                deadline = time.monotonic() + 2
                attempts = 1
                while not geometry["passed"] and time.monotonic() < deadline:
                    time.sleep(0.05)
                    geometry = placement_geometry(adapter, args.position)
                    attempts += 1
                geometry["settle_attempts"] = attempts
                geometry["window_state"] = adapter.run(["xprop", "-id", adapter.window, "WM_STATE", "_NET_WM_STATE"]).stdout.strip()
                (evidence / "geometry.json").write_text(json.dumps(geometry, indent=2) + "\n")
                assert geometry["passed"], "Compositor did not honor the requested initial position; inspect geometry.json"
                actions.append({"read": "Owned X11 geometry after activation, RandR primary monitor and EWMH workspace", "requested_position": args.position, "geometry": geometry})
            adapter.keys("Tab")
            time.sleep(0.2)
            adapter.capture(evidence / "02-after-tab.png")
            if args.feature == "countdown":
                # Tab is actual user input. The initial five-second fallback
                # must be suspended while the daemon's hard limit stays finite.
                time.sleep(5.2)
                adapter.capture(evidence / "03-countdown-review.png")
                ocr = adapter.run(["tesseract", str(evidence / "03-countdown-review.png"), "stdout"]).stdout
                assert "Reviewing" in ocr and "paused" in ocr, "Visible review status missing after ordinary timeout"
                actions.append({"input": "Tab; wait 5.2 seconds beyond ordinary countdown", "expected": "Owned permission popup remains visible with review status and request limit"})
            if args.decision == "allow":
                adapter.keys("Tab")
                time.sleep(0.2)
                adapter.capture(evidence / "03-after-second-tab.png")
            adapter.keys("space")
            actions.append({"input": "Tab; " + ("Tab; " if args.decision == "allow" else "") + "Space", "expected": args.decision + " once"})
            fixture.wait(timeout=12)
            report = json.loads((evidence / "fixture-report.json").read_text())
            if args.feature == "checksums":
                assert not report["commands"], "Opening checksum review must not submit policy changes"
                result["native_checksum_review"] = True
            latency = report["decisions"][0]["seconds"]
            assert report["decisions"][0]["action"] == args.decision and latency < (15 if args.feature == "countdown" else 10), "Prompt was not answered through input"
            if args.feature == "countdown":
                assert latency > 5, "Decision did not survive the initial five-second countdown"
                result["review_survived_seconds"] = latency
            adapter.attach(app, app.pid)
            adapter.capture(evidence / "02-after-decision.png")
        elif args.feature == "daemon-preferences":
            def steps(*keys):
                for key in keys:
                    adapter.keys(key)
                    time.sleep(0.25)
            time.sleep(1)
            steps("ctrl+4")
            time.sleep(0.5)
            adapter.capture(evidence / "00-daemon-nodes.png")
            ocr = adapter.run(["tesseract", str(evidence / "00-daemon-nodes.png"), "stdout"]).stdout
            assert "Synthetic local daemon" in ocr and "Interception enabled" in ocr, "Native configuration prerequisite: connected node metadata is not painted"
            steps("Return")  # search Enter selects the first visible node
            time.sleep(0.5)
            steps("Tab", "Tab", "Tab", "space")
            adapter.attach(app, app.pid, title="^Rustleblower — daemon configuration$")
            time.sleep(0.3)
            adapter.capture(evidence / "01-daemon-settings.png")
            steps("Return")
            adapter.type("Deny")
            steps("Return")
            time.sleep(0.3)
            adapter.capture(evidence / "02-daemon-default-action.png")
            steps("ctrl+Return")
            time.sleep(0.5)
            adapter.capture(evidence / "03-daemon-review.png")
            ocr = adapter.run(["tesseract", str(evidence / "03-daemon-review.png"), "stdout"]).stdout
            assert "Default action" in ocr and 'Before:' in ocr and 'After:' in ocr and "deny" in ocr.lower(), "Native changed-field review is missing"
            steps("ctrl+shift+Return")
            time.sleep(0.7)
            adapter.capture(evidence / "04-daemon-saved.png")
            ocr = adapter.run(["tesseract", str(evidence / "04-daemon-saved.png"), "stdout"]).stdout
            assert "confirmed and saved" in ocr, "Native configuration did not reach acknowledged success"
            actions.append({"input": "Ctrl+4; Enter selects first node; Tab to configuration; choose Deny; Ctrl+Enter reviews; Ctrl+Shift+Enter confirms", "expected": "Actual CHANGE_CONFIG acknowledgement persists only the edited default action in the isolated fixture"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature == "nodes":
            time.sleep(1)
            adapter.keys("ctrl+4")
            time.sleep(1)
            adapter.capture(evidence / "01-nodes.png")
            ocr = adapter.run(["tesseract", str(evidence / "01-nodes.png"), "stdout"]).stdout
            assert "Synthetic local daemon" in ocr and "Interception enabled" in ocr, "Native node list did not paint reported metadata"
            adapter.keys("ctrl+f")
            adapter.type("does-not-exist")
            time.sleep(0.5)
            adapter.capture(evidence / "02-nodes-empty.png")
            ocr = adapter.run(["tesseract", str(evidence / "02-nodes-empty.png"), "stdout"]).stdout
            assert "No matching cached nodes" in ocr, "Native node search did not filter metadata"
            actions.append({"input": "Ctrl+4 opens Nodes; Ctrl+F and type an absent label", "expected": "Reported node/interception paint, then No matching cached nodes"})
            fixture.wait(timeout=12)
        elif args.feature == "statistics":
            time.sleep(1)
            adapter.keys("ctrl+3")
            time.sleep(1)
            adapter.capture(evidence / "01-statistics.png")
            ocr = adapter.run(["tesseract", str(evidence / "01-statistics.png"), "stdout"]).stdout
            assert "Daemon totals" in ocr and "firefox" in ocr, "Native statistics view did not paint reported counters"
            adapter.keys("ctrl+f")
            adapter.type("does-not-exist")
            time.sleep(0.5)
            adapter.capture(evidence / "02-statistics-empty.png")
            ocr = adapter.run(["tesseract", str(evidence / "02-statistics-empty.png"), "stdout"]).stdout
            assert "No matching daemon counters" in ocr, "Native statistics search did not filter counters"
            actions.append({"input": "Ctrl+3 opens statistics; Ctrl+F and type an absent label", "expected": "Daemon counters and totals paint, then No matching daemon counters"})
            fixture.wait(timeout=12)
        elif args.feature == "history-tools":
            def steps(*keys):
                for key in keys:
                    adapter.keys(key)
                    time.sleep(0.2)  # focus/action dispatch needs a rendered frame

            # Ctrl+F gives a deterministic focus boundary. The next enabled
            # control is History tools (Details is disabled without selection).
            time.sleep(0.5)
            steps("ctrl+f", "Tab")
            time.sleep(0.2)
            adapter.capture(evidence / "01-history-tools-focus.png")
            steps("space")
            adapter.attach(app, app.pid, title="^Rustleblower — history tools$")
            adapter.capture(evidence / "02-history-tools.png")
            steps("Tab", "Return")
            time.sleep(0.3)
            adapter.capture(evidence / "03-field-open.png")
            adapter.type("UID")
            steps("Return")
            time.sleep(0.3)
            adapter.capture(evidence / "04-field-selected.png")
            steps("Tab", "Return")
            time.sleep(0.3)
            adapter.type("Equals")
            steps("Return")
            time.sleep(0.3)
            steps("Tab")
            adapter.capture(evidence / "05-comparison-selected.png")
            adapter.type("9999")
            adapter.capture(evidence / "06-value-typed.png")
            steps("Tab", "Return")
            time.sleep(0.3)
            adapter.capture(evidence / "07-added-condition.png")
            steps("Tab", "Tab", "space")
            time.sleep(0.5)
            adapter.capture(evidence / "03-history-filter-applied.png")
            adapter.attach(app, app.pid)
            time.sleep(0.5)
            adapter.capture(evidence / "04-history-filter-result.png")
            ocr = adapter.run(["tesseract", str(evidence / "04-history-filter-result.png"), "stdout"]).stdout
            assert "No matching events" in ocr, "Typed UID condition did not filter native history"
            actions.append({"input": "Ctrl+F; Tab/Space opens tools; choose UID/Equals, type 9999, Add and Apply through keyboard", "expected": "Actual metadata filter returns no matching events; fixture UID is 1000"})
            fixture.wait(timeout=12)
        else:
            time.sleep(1)
            adapter.capture(evidence / "01-history.png")
            adapter.keys("ctrl+f")
            adapter.type("does-not-exist")
            actions.append({"input": "Ctrl+F; type does-not-exist", "expected": "No matching events"})
            time.sleep(0.5)
            adapter.capture(evidence / "02-empty-search.png")
            ocr = adapter.run(["tesseract", str(evidence / "02-empty-search.png"), "stdout"]).stdout
            assert "No matching events" in ocr, "Visible empty-search state missing"
            adapter.keys("ctrl+a")
            adapter.type("firefox")
            time.sleep(0.5)
            adapter.capture(evidence / "03-filtered.png")
            fixture.wait(timeout=12)
        assert fixture.returncode == 0, "Synthetic fixture failed"
        result["status"] = "passed"
    except (Exception, KeyboardInterrupt) as error:
        result["error"] = str(error) or type(error).__name__
        if app is not None and app.poll() is None and adapter.window is not None:
            try:
                adapter.capture(evidence / "failure-state.png")
            except Exception as capture_error:
                result["failure_capture_error"] = str(capture_error)
    finally:
        fixture_exited = stop_owned_process(fixture)
        app_exited = stop_owned_process(app)
        cleanup = adapter.close()
        database = evidence / "history.sqlite3"
        if result["status"] == "passed":
            try:
                with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
                    result["persisted_rows"] = db.execute("SELECT COUNT(*) FROM events").fetchone()[0]
                    assert result["persisted_rows"] >= 430, "Fixture history was not persisted"
                    if args.feature == "daemon-preferences":
                        raw = db.execute("SELECT config FROM node_info WHERE name='Synthetic local daemon'").fetchone()[0]
                        assert json.loads(raw) == {"DefaultAction": "deny"}, "Native form defaulted or lost unedited settings"
                        report = json.loads((evidence / "fixture-report.json").read_text())
                        assert len(report["commands"]) == 1 and report["commands"][0]["type"] == 6 and report["commands"][0]["code"] == 0, "Expected one acknowledged CHANGE_CONFIG command (ui.proto action 6)"
                        assert report["commands"][0]["configuration"] == {"DefaultAction": "deny"}
                        result["persisted_daemon_configuration"] = {"DefaultAction": "deny"}
                    if args.feature == "nodes":
                        name, kernel, running, config_text = db.execute("SELECT name,kernel,running,config FROM node_info").fetchone()
                        assert (name, kernel, running) == ("Synthetic local daemon", "synthetic-kernel", 1), "Native node metadata was not persisted"
                        assert json.loads(config_text)["DefaultAction"] == "allow"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Node browsing must not change daemon policy"
                        result["persisted_nodes"] = 1
                    if args.feature == "statistics":
                        keys, hits = db.execute("SELECT count(*), sum(CAST(count AS INTEGER)) FROM statistics_counts WHERE kind=0").fetchone()
                        assert (keys, hits) == (3, 430), "Native counters were not persisted as absolute totals"
                        result["persisted_application_counters"] = keys
                        result["persisted_application_hits"] = hits
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Statistics viewing must not change daemon policy"
                    if args.feature in ("prompts", "placement", "countdown", "checksums"):
                        decisions = db.execute("SELECT COUNT(*) FROM events WHERE reason='decision' AND action=?", (args.decision,)).fetchone()[0]
                        assert decisions == 1, "Input decision was not persisted"
                        result["persisted_decisions"] = decisions
            except Exception as error:
                result.update(status="failed", error=str(error))
        scratch.cleanup()
        cleanup.update(owned_application_exited=app_exited, owned_fixture_exited=fixture_exited,
                       owned_socket_removed=not socket.exists())
        result["cleanup"] = cleanup
        if not all(cleanup.values()):
            result.update(status="failed", error="Owned resources remain")
        app_log.close()
        fixture_log.close()
        lock.close()
        (evidence / "actions.json").write_text(json.dumps(actions, indent=2) + "\n")
        (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
