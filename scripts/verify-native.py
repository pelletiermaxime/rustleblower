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


def tray_service(adapter, pid):
    deadline = time.monotonic() + 10
    prefix = f"org.kde.StatusNotifierItem-{pid}-"
    while time.monotonic() < deadline:
        adapter.alive()
        names = adapter.run(["busctl", "--user", "--no-pager", "--no-legend", "list"]).stdout.splitlines()
        names = [line.split()[0] for line in names if line.startswith(prefix)]
        if len(names) == 1:
            service = names[0]
            owner = adapter.run(["busctl", "--user", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", "s", service]).stdout.strip()
            assert owner == f"u {pid}", "Tray D-Bus service is not owned by the launched application"
            return service
        time.sleep(0.1)
    raise RuntimeError("Owned tray service did not register; require a working desktop StatusNotifier host")


def tray_call(adapter, service, method, *args, menu=False):
    path = "/MenuBar" if menu else "/StatusNotifierItem"
    interface = "com.canonical.dbusmenu" if menu else "org.kde.StatusNotifierItem"
    return adapter.run(["busctl", "--user", "--json=short", "--", "call", service, path, interface, method, *args]).stdout


def tray_menu_ids(adapter, service, evidence):
    layout = json.loads(tray_call(adapter, service, "GetLayout", "iias", "0", "-1", "0", menu=True))
    (evidence / "tray-menu.json").write_text(json.dumps(layout, indent=2) + "\n")
    labels = {}
    def visit(value):
        if isinstance(value, list):
            if len(value) == 3 and isinstance(value[0], int) and isinstance(value[1], dict):
                label = value[1].get("label", {})
                if isinstance(label, dict) and isinstance(label.get("data"), str):
                    labels[label["data"]] = value[0]
            for child in value:
                visit(child)
        elif isinstance(value, dict):
            for child in value.values():
                visit(child)
    visit(layout)
    assert "Open Rustleblower" in labels and "Quit Rustleblower" in labels, "Tray menu entries missing"
    return labels


def verify_tray_icons(adapter, service, evidence):
    raw = adapter.run(["busctl", "--user", "--json=short", "get-property", service,
                       "/StatusNotifierItem", "org.kde.StatusNotifierItem", "IconPixmap"]).stdout
    pixmaps = json.loads(raw)["data"]
    expected = json.loads((ROOT / "packaging/linux/icons/manifest.json").read_text())
    expected = {item["size"]: item for item in expected["sizes"] if item["size"] <= 32}
    assert len(pixmaps) == len(expected), "Tray did not publish all four size-specific drawings"
    report = []
    for width, height, pixels in pixmaps:
        assert width == height and width in expected, "Unexpected tray icon size"
        assert len(pixels) == width * height * 4, "Invalid tray ARGB payload size"
        checksum = hashlib.sha256(bytes(pixels)).hexdigest()
        assert checksum == expected[width]["argb_sha256"], "Tray pixels differ from the approved size-specific artwork"
        report.append({"size": width, "drawing": expected[width]["drawing"], "argb_sha256": checksum})
    assert {item["size"] for item in report} == set(expected)
    icon_name = adapter.run(["busctl", "--user", "--json=short", "get-property", service,
                            "/StatusNotifierItem", "org.kde.StatusNotifierItem", "IconName"]).stdout
    assert json.loads(icon_name)["data"] == "", "A theme name could override the size-specific tray drawings"
    (evidence / "tray-icons.json").write_text(json.dumps(report, indent=2) + "\n")
    return report


def owned_dashboard_windows(adapter):
    command = (["kdotool", "search", "--all", "--pid", str(adapter.pid), "--title", "^Rustleblower$"]
               if adapter.backend == "wayland" else ["xdotool", "search", "--all", "--pid", str(adapter.pid)])
    found = adapter.run(command, check=False)
    windows = found.stdout.split() if found.returncode == 0 else []
    assert len(windows) <= 32
    if adapter.backend == "x11":
        windows = [w for w in windows if adapter.run(["xdotool", "getwindowname", w], check=False).stdout.strip() == "Rustleblower"]
    return windows


def close_dashboard_to_tray(adapter):
    adapter.keys("alt+F4")
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        adapter.alive()
        if not owned_dashboard_windows(adapter):
            adapter.window = None
            return
        time.sleep(0.1)
    raise RuntimeError("Close minimized the dashboard instead of removing its desktop window")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["doctor", "run"])
    parser.add_argument("--backend", choices=["auto", "wayland", "x11"], default="auto")
    parser.add_argument("--display", default=os.environ.get("DISPLAY"))
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/rustleblower")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--decision", choices=["deny", "allow"], default="deny")
    parser.add_argument("--feature", choices=["tray", "history", "history-tools", "history-preferences", "firewall-forms", "firewall-policy", "subscriptions", "subscription-collections", "subscription-bulk", "subscription-transfer", "subscription-catalogue", "statistics", "nodes", "interception-preference", "daemon-preferences", "alerts", "live", "prompts", "placement", "countdown", "checksums", "window-state", "window-restore", "subscription-url", "subscription-inheritance"], default="history")
    parser.add_argument("--restore-state", type=Path)
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
        if args.feature in ("window-state", "window-restore"):
            assert backend == "wayland", "This owned compositor proof currently uses KDE Wayland"
            if args.feature == "window-restore":
                assert args.restore_state is not None, "Provide the prior owned window-state file"
                source=args.restore_state.resolve()
                prior=json.loads((source.parent / "result.json").read_text())
                assert prior["status"]=="passed" and prior["feature"]=="window-state" and all(prior["cleanup"].values())
                data=source.read_bytes()
                assert len(data)<=4096 and hashlib.sha256(data).hexdigest()==prior["saved_window_state_sha256"]
                restored=json.loads(data)
                assert restored["version"]==1 and restored["mode"]=="maximized" and restored["page"]=="nodes"
                (evidence / "history.sqlite3.window.json").write_bytes(data)
                result["restore_source"]=str(source)
                result["restore_source_sha256"]=hashlib.sha256(data).hexdigest()
                prior_geometry=json.loads((source.parent / "window-layout.json").read_text())
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
        if args.feature == "tray":
            service = tray_service(adapter, app.pid)
            result["tray_icons"] = verify_tray_icons(adapter, service, evidence)
            labels = tray_menu_ids(adapter, service, evidence)
            # Wait until registration reaches the UI and is painted.
            time.sleep(0.5)
            adapter.capture(evidence / "00-tray-ready.png")
            close_dashboard_to_tray(adapter)
            assert socket.exists(), "Server stopped when the dashboard closed"
            tray_call(adapter, service, "Activate", "ii", "0", "0")
            adapter.attach(app, app.pid)
            adapter.capture(evidence / "00-tray-activated.png")
            close_dashboard_to_tray(adapter)
            actions.append({"input": "Owned dashboard Alt+F4; actual tray Activate RPC; Alt+F4 again", "expected": "Dashboard surface destroyed each time; tray activation restores it; server remains alive"})
        fixture = subprocess.Popen(["python", str(ROOT / "scripts/fake-daemon.py"), "--socket", str(socket),
            "--expected-duration", "always" if args.decision == "allow" else "once",
            "--seconds", "30" if args.feature in ("window-state", "window-restore") else "45" if args.feature in ("subscription-bulk","subscription-transfer","subscription-catalogue") else "30" if args.feature in ("interception-preference", "daemon-preferences", "live", "history-preferences", "firewall-forms", "firewall-policy", "subscriptions", "subscription-collections", "subscription-bulk") else "5", "--prompts", "1" if args.feature in ("tray", "prompts", "placement", "countdown", "checksums") else "0",
            "--checksum-rules", "1" if args.feature == "checksums" else "0", "--report", str(evidence / "fixture-report.json")] + (["--allow-node-config"] if args.feature == "daemon-preferences" else []) + (["--alerts","6"] if args.feature=="alerts" else []) + (["--allow-live-tasks"] if args.feature=="live" else []) + (["--allow-system-firewall"] if args.feature in ("firewall-forms","firewall-policy") else []) + (["--system-firewall-drop"] if args.feature=="firewall-policy" else []),
            env=env, stdout=fixture_log, stderr=fixture_log)
        if args.feature in ("tray", "prompts", "placement", "countdown", "checksums"):
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
            # Kit scope radios precede Block. Placement deliberately preserves
            # the legacy connection scope, which adds a third Custom radio.
            block_tabs = 4 if args.feature == "placement" else 3
            for _ in range(block_tabs):
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
                adapter.keys("Tab")
                time.sleep(0.2)
                adapter.capture(evidence / "03-allow-forever-focused.png")
            adapter.keys("space")
            actions.append({"input": f"Tab × {block_tabs + (2 if args.decision == 'allow' else 0)}; Space", "expected": "allow forever" if args.decision == "allow" else "deny once"})
            fixture.wait(timeout=12)
            report = json.loads((evidence / "fixture-report.json").read_text())
            if args.feature == "checksums":
                assert not report["commands"], "Opening checksum review must not submit policy changes"
                result["native_checksum_review"] = True
            latency = report["decisions"][0]["seconds"]
            assert report["decisions"][0]["duration"] == ("always" if args.decision == "allow" else "once"), "Unexpected prompt duration"
            assert report["decisions"][0]["action"] == args.decision and latency < (15 if args.feature == "countdown" else 10), "Prompt was not answered through input"
            if args.feature == "countdown":
                assert latency > 5, "Decision did not survive the initial five-second countdown"
                result["review_survived_seconds"] = latency
            if args.feature == "tray":
                assert not owned_dashboard_windows(adapter), "Permission popup restored the hidden dashboard"
                tray_call(adapter, service, "Event", "isvu", str(labels["Open Rustleblower"]), "clicked", "s", "", "0", menu=True)
            adapter.attach(app, app.pid)
            adapter.capture(evidence / "02-after-decision.png")
            if args.feature == "tray":
                tray_call(adapter, service, "Event", "isvu", str(labels["Quit Rustleblower"]), "clicked", "s", "", "0", menu=True)
                assert app.wait(timeout=10) == 0, "Tray Quit did not exit cleanly"
                result["tray_lifecycle"] = {"close_destroyed_dashboard": True, "activate_restored": True, "permission_while_hidden": True, "menu_open_restored": True, "menu_quit_exited": True}
                actions.append({"input": "Answer independent permission by keyboard while dashboard is absent; actual dbusmenu Open and Quit events", "expected": "Real gRPC decision before fallback; recreated dashboard; clean application exit"})
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
            steps("Tab", "Tab", "Tab", "Tab", "space")
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
        elif args.feature == "subscription-collections":
            time.sleep(1)
            adapter.keys("ctrl+7")
            time.sleep(0.4)
            adapter.keys("ctrl+shift+l")
            time.sleep(0.6)
            adapter.capture(evidence / "01-native-empty-collections.png")
            ocr=adapter.run(["tesseract",str(evidence / "01-native-empty-collections.png"),"stdout"]).stdout
            assert "Rule collections" in ocr and "All enabled subscriptions" in ocr and "0 active sources" in ocr, "Native empty collection page missing"
            # Search -> three sort buttons -> descending checkbox -> first review.
            for _ in range(5):
                adapter.keys("Tab")
                time.sleep(0.15)
            adapter.keys("space")
            adapter.attach(app,app.pid,title="^Rustleblower — application rule$")
            time.sleep(0.4)
            adapter.keys("Tab")  # Move the caret out of the name before OCR.
            time.sleep(0.2)
            adapter.capture(evidence / "02-native-collection-rule-review.png")
            ocr=adapter.run(["tesseract",str(evidence / "02-native-collection-rule-review.png"),"stdout"]).stdout
            (evidence / "02-native-collection-rule-review.txt").write_text(ocr)
            normalized=ocr.replace("—","-").replace("–","-")
            assert "group-all-domains" in normalized and "User ID" in ocr and str(os.getuid()) in ocr, "Native collection draft lost its kind or local UID"
            # The retained evidence path is longer than this single-line input.
            # Name -> description/metadata -> first condition -> second directory.
            for _ in range(16):
                adapter.keys("Tab")
                time.sleep(0.12)
            adapter.keys("End")
            time.sleep(0.3)
            adapter.capture(evidence / "03-native-collection-directory-tail.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-native-collection-directory-tail.png"),"stdout"]).stdout
            (evidence / "03-native-collection-directory-tail.txt").write_text(ocr)
            assert "collections/current/all/domains" in ocr, "Native directory input does not expose the exact typed collection target through keyboard navigation"
            # Window-manager close intentionally minimizes this editor. Use its
            # actual Cancel button: directory -> Remove -> Add condition -> Cancel.
            for _ in range(3):
                adapter.keys("Tab")
                time.sleep(0.15)
            adapter.keys("space")
            adapter.attach(app,app.pid,title="^Rustleblower$")
            time.sleep(0.5)
            adapter.keys("ctrl+f")
            adapter.type("absent collection fixture")
            time.sleep(0.6)
            adapter.capture(evidence / "04-native-filtered-collections.png")
            ocr=adapter.run(["tesseract",str(evidence / "04-native-filtered-collections.png"),"stdout"]).stdout
            assert "No matching collections" in ocr, "Ctrl+F did not target the visible collection search after closing review"
            actions.append({"input":"Ctrl+7; Ctrl+Shift+L; Tab to first collection review; Space; Tab to Cancel; Space cancels; Ctrl+F and absent query", "expected":"Usable empty all-list page, reviewed local UID AND domain directory, no policy command, correct restored search focus and owned cleanup"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature == "subscription-inheritance":
            def key(key):
                adapter.keys(key)
                time.sleep(0.2)
            time.sleep(1)
            key("ctrl+7")
            key("ctrl+n")
            adapter.type("Native inherited limits")
            key("Tab")
            adapter.type_url("http://127.0.0.1:9/owned-native-inheritance")
            adapter.capture(evidence / "01-native-inheritance-draft.png")
            for _ in range(4):
                key("Tab")
            for index,unit in enumerate(("minutes","seconds","KB")):
                key("Return")
                adapter.type(unit)
                time.sleep(0.3)
                key("Return")
                if index<2:
                    key("Tab")
                    key("Tab")
            adapter.capture(evidence / "02-native-inheritance-units.png")
            key("ctrl+s")
            time.sleep(0.8)
            adapter.capture(evidence / "03-native-inheritance-saved.png")
            text=adapter.run(["tesseract",str(evidence / "03-native-inheritance-saved.png"),"stdout"]).stdout
            (evidence / "03-native-inheritance-saved.txt").write_text(text)
            assert "Native inherited limits" in text and "Subscription saved" in text,"Native count/unit controls did not save"
            actions.append({"input":"Ctrl+7; Ctrl+N; name/URL; empty counts with minutes/seconds/KB selected independently; Ctrl+S","expected":"Persisted unit-only overrides retain null counts and canonical units without downloads or policy changes"})
            fixture.wait(timeout=12)
        elif args.feature == "subscription-url":
            time.sleep(1)
            adapter.keys("ctrl+7")
            time.sleep(0.4)
            adapter.keys("ctrl+n")
            time.sleep(0.4)
            adapter.keys("Tab")
            time.sleep(0.2)
            adapter.type_url("file:///owned/url-test-must-not-fetch")
            adapter.capture(evidence / "01-native-url-test-form.png")
            for _ in range(13):
                adapter.keys("Tab")
                time.sleep(0.15)
            adapter.capture(evidence / "02-native-url-test-focus.png")
            adapter.keys("space")
            time.sleep(0.7)
            adapter.capture(evidence / "03-native-url-test-refused.png")
            text=adapter.run(["tesseract",str(evidence / "03-native-url-test-refused.png"),"stdout"]).stdout
            (evidence / "03-native-url-test-refused.txt").write_text(text)
            assert "HTTP(S) URL without embedded credentials" in text, "Native URL test did not refuse the unsupported scheme before I/O"
            actions.append({"input":"Ctrl+7; Ctrl+N; Tab/type file URI; thirteen Tabs; Space on Test URL","expected":"Actual native URL diagnostic refuses unsupported scheme; no saved subscription, list download or policy command"})
            fixture.wait(timeout=12)
        elif args.feature in ("subscriptions","subscription-bulk"):
            time.sleep(1)
            adapter.keys("ctrl+7")
            time.sleep(0.5)
            adapter.capture(evidence / "01-native-empty-subscriptions.png")
            ocr=adapter.run(["tesseract",str(evidence / "01-native-empty-subscriptions.png"),"stdout"]).stdout
            assert "Blocklist subscriptions" in ocr and "No subscriptions yet" in ocr, "Native empty subscription entry is missing"
            adapter.keys("ctrl+n")
            time.sleep(0.4)
            adapter.type("Native blocklist fixture")
            adapter.keys("Tab")
            time.sleep(0.25)
            adapter.type_url("http://127.0.0.1:9/owned-native-list")
            adapter.capture(evidence / "02-native-subscription-form.png")
            adapter.keys("ctrl+s")
            time.sleep(0.8)
            adapter.capture(evidence / "03-native-saved-subscription.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-native-saved-subscription.png"),"stdout"]).stdout
            (evidence / "03-native-saved-subscription.txt").write_text(ocr)
            assert "Native blocklist fixture" in ocr and "Subscription saved" in ocr and "paused" in ocr, "Native subscription input was not saved while paused"
            actions.append({"input":"Ctrl+7; Ctrl+N; type name; Tab; type URL; Ctrl+S", "expected":"UI creates persisted subscription metadata with automatic updates paused; no HTTP request or daemon policy command"})
            if args.feature == "subscription-bulk":
                adapter.keys("ctrl+shift+b")
                time.sleep(0.4)
                adapter.capture(evidence / "03a-native-bulk-fields.png")
                adapter.keys("Return")
                time.sleep(0.4)
                adapter.type("Disable")
                time.sleep(0.4)
                adapter.capture(evidence / "03b-native-bulk-status-search.png")
                adapter.keys("Return")
                time.sleep(0.4)
                adapter.capture(evidence / "03c-native-bulk-status-selected.png")
                adapter.keys("ctrl+Return")
                time.sleep(0.6)
                adapter.capture(evidence / "04-native-bulk-review.png")
                ocr=adapter.run(["tesseract",str(evidence / "04-native-bulk-review.png"),"stdout"]).stdout
                (evidence / "04-native-bulk-review.txt").write_text(ocr)
                assert "1 examined" in ocr and "1 changes" in ocr and "Enabled: true" in ocr and "false" in ocr, "Native bulk preview lacks the exact enabled-state change"
                adapter.keys("ctrl+shift+Return")
                time.sleep(0.7)
                adapter.capture(evidence / "05-native-bulk-applied.png")
                ocr=adapter.run(["tesseract",str(evidence / "05-native-bulk-applied.png"),"stdout"]).stdout
                (evidence / "05-native-bulk-applied.txt").write_text(ocr)
                assert "Updated 1 subscriptions together" in ocr and "disabled" in ocr, "Native reviewed bulk change was not applied"
                actions.append({"input":"Ctrl+Shift+B; Return; Disable; Return; Ctrl+Return review; Ctrl+Shift+Return apply", "expected":"One reviewed enabled-state change saved through real native keyboard input; retained URL and limits; no policy command or HTTP download"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature == "subscription-transfer":
            source=evidence / "native-import-action.json"
            source.write_text(json.dumps({"name":"ownedNativeAction","actions":{"other_plugin":{"enabled":True,"marker":"never execute"},"list_subscriptions":{"enabled":False,"config":{"lists_dir":"/not/adopted/by/rustleblower","notify":{"success":{"desktop":"Owned metadata"}},"subscriptions":[{"name":"Native imported fixture","url":"http://127.0.0.1:9/owned-native-import","filename":"native-ips.txt","format":"ips","enabled":False,"group":"Native Test","interval":2,"interval_units":"minutes","timeout":3,"timeout_units":"seconds","max_size":2,"max_size_units":"KB"}]}}}},indent=2)+"\n")
            time.sleep(1)
            adapter.keys("ctrl+7")
            time.sleep(0.5)
            adapter.keys("ctrl+shift+t")
            time.sleep(0.5)
            adapter.type_url(str(source))
            time.sleep(0.4)
            adapter.capture(evidence / "01-native-import-path.png")
            adapter.keys("ctrl+Return")
            time.sleep(0.7)
            adapter.capture(evidence / "02-native-import-review.png")
            ocr=adapter.run(["tesseract",str(evidence / "02-native-import-review.png"),"stdout"]).stdout
            (evidence / "02-native-import-review.txt").write_text(ocr)
            assert "Native imported fixture" in ocr and "New subscription" in ocr and "0 conflicts" in ocr, "Native import review lost its exact proposed subscription"
            adapter.keys("ctrl+shift+Return")
            time.sleep(0.8)
            adapter.capture(evidence / "03-native-import-applied.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-native-import-applied.png"),"stdout"]).stdout
            (evidence / "03-native-import-applied.txt").write_text(ocr)
            assert "Imported 1 subscription changes" in ocr and "Native imported fixture" in ocr and "disabled" in ocr, "Native import did not reach saved feedback"
            actions.append({"input":"Ctrl+7; Ctrl+Shift+T; owned source path; Ctrl+Return review; Ctrl+Shift+Return apply","expected":"One converted legacy subscription, unchanged defaults, paused/no-download state, retained metadata, no implicit policy command and owned cleanup"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature == "subscription-catalogue":
            source=evidence / "native-catalogue.json"
            source.write_text(json.dumps({"actions":{"list_subscriptions":{"enabled":False,"config":{"subscriptions":[{"name":"Shared native","url":"http://127.0.0.1:9/native-first","filename":"native-first-ips.txt","format":"ips","enabled":False,"future_tag":"first"},{"name":"Shared native","url":"http://127.0.0.1:9/native-second","filename":"native-second-ips.txt","format":"ips","enabled":False,"future_tag":"second"}]}}}},indent=2)+"\n")
            time.sleep(1)
            adapter.keys("ctrl+7")
            time.sleep(0.4)
            adapter.keys("ctrl+n")
            time.sleep(0.4)
            adapter.type("Retained native fixture")
            adapter.keys("Tab")
            time.sleep(0.2)
            adapter.type_url("http://127.0.0.1:9/retained-native")
            adapter.keys("ctrl+s")
            time.sleep(0.6)
            adapter.capture(evidence / "01-native-before-catalogue.png")
            adapter.keys("ctrl+shift+t")
            time.sleep(0.4)
            adapter.type_url(str(source))
            for _ in range(2):
                adapter.keys("Tab")
                time.sleep(0.2)
            adapter.keys("space")
            time.sleep(0.2)
            adapter.keys("ctrl+Return")
            time.sleep(0.8)
            adapter.capture(evidence / "02-native-catalogue-review.png")
            ocr=adapter.run(["tesseract",str(evidence / "02-native-catalogue-review.png"),"stdout"]).stdout
            (evidence / "02-native-catalogue-review.txt").write_text(ocr)
            assert "2 entries" in ocr and "3 changes" in ocr and "1 retained removals" in ocr and "Complete catalogue replacement" in ocr, "Native replacement review must include both new identities and the exact retained removal"
            adapter.keys("ctrl+shift+Return")
            time.sleep(0.8)
            adapter.capture(evidence / "03-native-catalogue-applied.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-native-catalogue-applied.png"),"stdout"]).stdout
            (evidence / "03-native-catalogue-applied.txt").write_text(ocr)
            assert "Imported 3 subscription changes" in ocr and "Shared native" in ocr, "Native complete replacement did not reach saved feedback"
            actions.append({"input":"Create owned old subscription; Ctrl+Shift+T; file path; Tab twice; Space selects complete replacement; Ctrl+Return review; Ctrl+Shift+Return apply","expected":"Two same-name new identities and one retained removal, with exact per-ID extra fields and no download or policy command"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature == "firewall-policy":
            def steps(*keys):
                for key in keys:
                    adapter.keys(key)
                    actions.append({"input":key,"expected":"Owned reviewed system-firewall state transition"})
                    time.sleep(0.25)
            time.sleep(1)
            steps("ctrl+4","Return")
            time.sleep(0.5)
            steps("Tab","Tab","Tab","Tab","space")
            adapter.attach(app,app.pid,title="^Rustleblower — system firewall$")
            steps("alt+p")
            time.sleep(0.5)
            adapter.capture(evidence / "01-native-policies.png")
            ocr=adapter.run(["tesseract",str(evidence / "01-native-policies.png"),"stdout"]).stdout
            assert "Policies and state" in ocr and "drop" in ocr,"Native policy pane or captured drop policy missing"
            steps("ctrl+shift+e")
            time.sleep(0.5)
            adapter.capture(evidence / "02-staged-native-disable.png")
            steps("ctrl+Return")
            time.sleep(0.5)
            adapter.capture(evidence / "03-native-transition-review.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-native-transition-review.png"),"stdout"]).stdout
            (evidence / "03-native-transition-review.txt").write_text(ocr)
            assert "Prepare accept policies" in ocr and "Step 2/2" in ocr and "Save the disabled" in ocr and "accept" in ocr,"Native complete two-stage review missing"
            steps("ctrl+shift+Return")
            time.sleep(1)
            adapter.capture(evidence / "04-native-disable-accepted.png")
            ocr=adapter.run(["tesseract",str(evidence / "04-native-disable-accepted.png"),"stdout"]).stdout
            assert "enabled false" in ocr.lower() and "accepted reload" in ocr,"Native reviewed disable did not finish"
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
            actions.append({"input":"Open node firewall; Alt+P; Ctrl+Shift+E stages disable; Ctrl+Enter reviews both stages; Ctrl+Shift+Enter applies", "expected":"Two exact accepted in-memory reloads (active accept policies, then disabled), durable final cache and cleanup"})
        elif args.feature == "firewall-forms":
            def steps(*keys):
                for key in keys:
                    adapter.keys(key)
                    actions.append({"input":key,"expected":"Owned system-firewall keyboard step"})
                    time.sleep(0.25)
            time.sleep(1)
            steps("ctrl+4", "Return")
            time.sleep(0.5)
            steps("Tab", "Tab", "Tab", "Tab", "space")
            adapter.attach(app, app.pid, title="^Rustleblower — system firewall$")
            time.sleep(0.5)
            adapter.capture(evidence / "01-system-firewall.png")
            ocr=adapter.run(["tesseract",str(evidence / "01-system-firewall.png"),"stdout"]).stdout
            assert "fixture-input" in ocr and "nftables" in ocr,"Native firewall snapshot is not painted"
            steps("alt+e")
            time.sleep(0.5)
            steps("ctrl+a")
            adapter.type("fixture-input-native")
            adapter.capture(evidence / "02-chain-form.png")
            steps("ctrl+Return")
            time.sleep(0.5)
            steps("ctrl+Return")
            time.sleep(0.5)
            adapter.capture(evidence / "03-firewall-review.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-firewall-review.png"),"stdout"]).stdout
            assert "Name" in ocr and "fixture-input-native" in ocr,"Native firewall before/after review is missing"
            steps("ctrl+shift+Return")
            time.sleep(0.7)
            adapter.capture(evidence / "04-firewall-accepted.png")
            ocr=adapter.run(["tesseract",str(evidence / "04-firewall-accepted.png"),"stdout"]).stdout
            assert "accepted reload" in ocr and "fixture-input-native" in ocr,"Native firewall reload did not reach accepted feedback"
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
            actions.append({"input":"Select node, open system firewall, Alt+E edits chain, Ctrl+Enter keeps and reviews, Ctrl+Shift+Enter applies", "expected":"One exact complete synthetic firewall reload, only chain name changed, durable accepted marker and complete cleanup"})
        elif args.feature == "live":
            time.sleep(1)
            adapter.keys("ctrl+6")
            time.sleep(0.5)
            adapter.keys("ctrl+Return")
            time.sleep(1)
            adapter.capture(evidence / "01-live-sockets.png")
            ocr = adapter.run(["tesseract",str(evidence / "01-live-sockets.png"),"stdout"]).stdout
            assert "synthetic-live" in ocr and "TCP" in ocr, "Native live socket data was not painted"
            adapter.keys("ctrl+f")
            adapter.type("does-not-exist")
            time.sleep(0.7)
            adapter.capture(evidence / "02-live-empty.png")
            ocr = adapter.run(["tesseract",str(evidence / "02-live-empty.png"),"stdout"]).stdout
            assert "No matching live sockets" in ocr, "Native live search did not reach empty state"
            adapter.keys("ctrl+shift+Return")
            time.sleep(0.7)
            adapter.capture(evidence / "03-live-stop-requested.png")
            ocr = adapter.run(["tesseract",str(evidence / "03-live-stop-requested.png"),"stdout"]).stdout
            assert "Stop requested" in ocr, "Native live pause did not request task stop"
            actions.append({"input":"Ctrl+6; Ctrl+Enter starts; Ctrl+F types an absent query; Ctrl+Shift+Enter pauses", "expected":"Synthetic socket rows, empty search, exact TASK_START/TASK_STOP without invented stop ACK"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature == "alerts":
            time.sleep(1)
            adapter.keys("ctrl+5")
            time.sleep(1)
            adapter.capture(evidence / "01-alerts.png")
            ocr=adapter.run(["tesseract",str(evidence / "01-alerts.png"),"stdout"]).stdout
            assert "Synthetic daemon alert" in ocr and "Warning" in ocr,"Native alert rows were not painted"
            adapter.keys("ctrl+f")
            adapter.type("does-not-exist")
            time.sleep(0.5)
            adapter.capture(evidence / "02-alerts-empty.png")
            ocr=adapter.run(["tesseract",str(evidence / "02-alerts-empty.png"),"stdout"]).stdout
            assert "No matching retained alerts" in ocr,"Native alert search did not reach empty state"
            actions.append({"input":"Ctrl+5; Ctrl+F; type absent text","expected":"Reported alerts painted, then empty-state search; six rows persisted without policy commands"})
            fixture.wait(timeout=12)
        elif args.feature == "interception-preference":
            time.sleep(1)
            adapter.keys("ctrl+4")
            time.sleep(0.5)
            adapter.keys("Return")
            time.sleep(0.6)
            adapter.capture(evidence / "01-native-pause-preference-off.png")
            ocr=adapter.run(["tesseract",str(evidence / "01-native-pause-preference-off.png"),"stdout"]).stdout
            assert "Remember this node" in ocr and "Off by default" in ocr, "Selected native node did not expose its opt-in pause control"
            adapter.keys("Tab")
            time.sleep(0.2)
            adapter.keys("space")
            time.sleep(0.8)
            adapter.capture(evidence / "02-native-pause-preference-saved.png")
            ocr=adapter.run(["tesseract",str(evidence / "02-native-pause-preference-saved.png"),"stdout"]).stdout
            assert "Saved choice: running" in ocr and "Saved this node" in ocr, "Native checkbox did not reach persisted pause preference"
            actions.append({"input":"Ctrl+4; Enter selects first node; Tab; Space checks Remember paused state", "expected":"Opt-in saved for exact current node without changing daemon interception"})
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
        elif args.feature == "history-preferences":
            def steps(*keys):
                for key in keys:
                    adapter.keys(key)
                    actions.append({"input":key,"expected":"Owned history-preferences keyboard step"})
                    time.sleep(0.2)
            time.sleep(0.5)
            adapter.attach(app,app.pid)
            steps("ctrl+f", "Tab", "Tab", "space")
            adapter.attach(app, app.pid, title="^Settings$")
            adapter.capture(evidence / "01-history-preferences.png")
            steps("ctrl+a")
            adapter.type("1")
            steps("ctrl+Return")
            time.sleep(0.5)
            adapter.capture(evidence / "02-retention-review.png")
            ocr=adapter.run(["tesseract",str(evidence / "02-retention-review.png"),"stdout"]).stdout
            assert "Review retention changes" in ocr, "Native retention did not reach review"
            steps("ctrl+shift+Return")
            time.sleep(0.8)
            adapter.capture(evidence / "03-retention-saved.png")
            ocr=adapter.run(["tesseract",str(evidence / "03-retention-saved.png"),"stdout"]).stdout
            assert "Retention saved" in ocr, "Native retention did not save"
            actions.append({"input":"Ctrl+F; Tab/Tab/Space opens preferences; replace days with 1; Ctrl+Enter reviews; Ctrl+Shift+Enter applies", "expected":"Native review/save feedback; one-day retention independently persisted; no daemon policy commands"})
            fixture.send_signal(signal.SIGTERM)
            fixture.wait(timeout=12)
        elif args.feature in ("window-state", "window-restore"):
            state_path=evidence / "history.sqlite3.window.json"
            def read_layout(mode, page="nodes"):
                deadline=time.monotonic()+8
                while time.monotonic()<deadline:
                    adapter.assert_focus()
                    try:
                        state=json.loads(state_path.read_text())
                        if state["mode"]==mode and state["page"]==page:
                            return state
                    except (FileNotFoundError,json.JSONDecodeError):
                        pass
                    time.sleep(0.1)
                raise RuntimeError("Actual compositor state was not saved")
            def geometry():
                raw=adapter.run(["kdotool","getwindowgeometry",adapter.window]).stdout
                position=re.search(r"Position:\s*(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?)",raw)
                dimensions=re.search(r"Geometry:\s*(\d+(?:\.\d+)?)x(\d+(?:\.\d+)?)",raw)
                assert position and dimensions, f"Unexpected owned window geometry: {raw}"
                return dict(zip(("x","y","width","height"),map(float,position.groups()+dimensions.groups())))
            if args.feature == "window-state":
                time.sleep(0.5)
                adapter.keys("ctrl+4")
                adapter.run(["kdotool","windowsize",adapter.window,"1000","720"])
                time.sleep(0.9)
                normal=read_layout("windowed")
                normal_geometry=geometry()
                adapter.capture(evidence / "01-windowed-nodes.png")
                adapter.run(["kdotool","windowstate","--add","MAXIMIZED",adapter.window])
                time.sleep(0.9)
                maximized=read_layout("maximized")
                maximized_geometry=geometry()
                assert maximized_geometry["width"]>=normal_geometry["width"] and maximized_geometry["height"]>=normal_geometry["height"]
                assert max(maximized_geometry[k]-normal_geometry[k] for k in ("width","height"))>8, "Maximize did not expand the owned native surface"
                adapter.capture(evidence / "02-maximized-nodes.png")
                layout={"normal":normal,"normal_geometry":normal_geometry,"maximized":maximized,"maximized_geometry":maximized_geometry}
                actions.append({"input":"Ctrl+4; owned compositor windowsize 1000x720; maximize owned window", "expected":"Actual size and Nodes page saved, real native maximized state and expanded geometry"})
            else:
                time.sleep(1.5)
                maximized=read_layout("maximized")
                maximized_geometry=geometry()
                for key in ("width","height"):
                    assert abs(maximized_geometry[key]-prior_geometry["maximized_geometry"][key])<=3, "Restored maximized native geometry differs"
                adapter.capture(evidence / "01-restored-maximized-nodes.png")
                ocr=adapter.run(["tesseract",str(evidence / "01-restored-maximized-nodes.png"),"stdout"]).stdout
                assert "choose a node" in ocr.lower() and "interception" in ocr.lower(), "Last Nodes page was not restored natively"
                adapter.run(["kdotool","windowstate","--remove","MAXIMIZED",adapter.window])
                time.sleep(0.9)
                normal=read_layout("windowed")
                normal_geometry=geometry()
                for key in ("width","height"):
                    assert abs(normal_geometry[key]-prior_geometry["normal_geometry"][key])<=3, "Native restore size was lost across restart"
                adapter.capture(evidence / "02-unmaximized-restored-size.png")
                layout={"normal":normal,"normal_geometry":normal_geometry,"maximized":maximized,"maximized_geometry":maximized_geometry}
                actions.append({"input":"Fresh owned process with exact previously saved file; native unmaximize", "expected":"Nodes page and maximized surface restored, original windowed size recovered"})
            (evidence / "window-layout.json").write_text(json.dumps(layout,indent=2)+"\n")
            result["window_layout"]=layout
            fixture.send_signal(signal.SIGTERM)
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
                    if args.feature == "subscription-collections":
                        with sqlite3.connect((evidence / "subscriptions/subscriptions.sqlite3").as_uri() + "?mode=ro", uri=True) as subscription_db:
                            assert subscription_db.execute("SELECT count(*) FROM subscriptions").fetchone()[0]==0, "Empty native collection setup must not bootstrap subscription config"
                            assert not json.loads(subscription_db.execute("SELECT config FROM settings WHERE id=1").fetchone()[0])["automatic"], "Native collection setup must leave downloads paused"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Cancelled native collection review sent policy commands"
                        result["collection_review"]={"uid":os.getuid(),"kind":"domains","members":0,"sent_commands":0}
                    if args.feature in ("subscriptions","subscription-bulk"):
                        with sqlite3.connect((evidence / "subscriptions/subscriptions.sqlite3").as_uri() + "?mode=ro", uri=True) as subscription_db:
                            rows=subscription_db.execute("SELECT config,status,bytes,file,checked FROM subscriptions").fetchall()
                            assert len(rows)==1, "Native input must create exactly one subscription"
                            config=json.loads(rows[0][0])
                            assert config=={"name":"Native blocklist fixture","url":"http://127.0.0.1:9/owned-native-list","filename":"subscription.txt","format":"hosts","enabled":args.feature!="subscription-bulk","groups":[],"interval":None,"timeout":None,"max_bytes":None}, "Native form or bulk change defaulted or lost fields"
                            assert rows[0][1:]==("changed" if args.feature=="subscription-bulk" else "never",0,None,0), "Metadata-only native proof must not start a download"
                            settings=json.loads(subscription_db.execute("SELECT config FROM settings WHERE id=1").fetchone()[0])
                            assert settings["automatic"] is False, "Native initial setup must keep automatic updates paused"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Subscription setup issued a daemon policy command"
                        result["persisted_subscription"]={"name":config["name"],"automatic":False,"downloaded_bytes":0,"format":"hosts","enabled":config["enabled"]}
                    if args.feature == "subscription-transfer":
                        with sqlite3.connect((evidence / "subscriptions/subscriptions.sqlite3").as_uri()+"?mode=ro",uri=True) as subscriptions_db:
                            rows=subscriptions_db.execute("SELECT config,checked,bytes,file FROM subscriptions").fetchall()
                            assert len(rows)==1 and rows[0][1:]==(0,0,None), "Native import must remain metadata-only"
                            config=json.loads(rows[0][0])
                            assert config=={"name":"Native imported fixture","url":"http://127.0.0.1:9/owned-native-import","filename":"native-ips.txt","format":"ips","enabled":False,"groups":["native-test"],"interval":{"count":2,"unit":"minutes"},"timeout":{"count":3,"unit":"seconds"},"max_bytes":{"count":2,"unit":"KB"}}, "Native migration lost fields or unit/group conversion"
                            settings=json.loads(subscriptions_db.execute("SELECT config FROM settings WHERE id=1").fetchone()[0])
                            assert settings["automatic"] is False and settings["interval"]==86400 and settings["timeout"]==60, "Unchecked defaults must remain unchanged"
                            template=json.loads(subscriptions_db.execute("SELECT template FROM subscription_interchange WHERE id=1").fetchone()[0])
                            assert template["actions"]["other_plugin"]["marker"]=="never execute" and template["actions"]["list_subscriptions"]["config"]["notify"]["success"]["desktop"]=="Owned metadata", "Native import discarded action metadata"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Native import issued a policy command"
                        result["persisted_import"]={"configuration":config,"automatic":False,"downloaded_bytes":0,"checked":0,"policy_commands":0}
                    if args.feature == "subscription-catalogue":
                        with sqlite3.connect((evidence / "subscriptions/subscriptions.sqlite3").as_uri()+"?mode=ro",uri=True) as subscription_db:
                            rows=subscription_db.execute("SELECT id,config,removed,checked,bytes,file FROM subscriptions ORDER BY id").fetchall()
                            assert len(rows)==3 and [row[2] for row in rows]==[1,0,0], "Native replacement lost its retained row or imported identities"
                            assert all(row[3:]==(0,0,None) for row in rows), "Native replacement must be metadata-only"
                            configs=[json.loads(row[1]) for row in rows]
                            assert configs[0]["name"]=="Retained native fixture" and all(row["name"]=="Shared native" and row["format"]=="ips" and row["enabled"] is False for row in configs[1:])
                            assert [row["url"] for row in configs[1:]]==["http://127.0.0.1:9/native-first","http://127.0.0.1:9/native-second"]
                            extras=subscription_db.execute("SELECT subscription,record FROM subscription_source_metadata ORDER BY subscription").fetchall()
                            assert [(row[0],json.loads(row[1])) for row in extras]==[(2,{"future_tag":"first"}),(3,{"future_tag":"second"})], "Same-name identities exchanged their extra fields"
                            settings=json.loads(subscription_db.execute("SELECT config FROM settings WHERE id=1").fetchone()[0])
                            assert settings["automatic"] is False and settings["interval"]==86400 and settings["timeout"]==60, "Unselected global defaults changed during native replacement"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Native replacement issued a policy command"
                        result["persisted_catalogue"]={"active":2,"retained_removed":1,"names":["Shared native","Shared native"],"source_metadata_ids":[2,3],"automatic":False,"downloaded_bytes":0,"policy_commands":0}
                    if args.feature == "daemon-preferences":
                        raw = db.execute("SELECT config FROM node_info WHERE name='Synthetic local daemon'").fetchone()[0]
                        assert json.loads(raw) == {"DefaultAction": "deny"}, "Native form defaulted or lost unedited settings"
                        report = json.loads((evidence / "fixture-report.json").read_text())
                        assert len(report["commands"]) == 1 and report["commands"][0]["type"] == 6 and report["commands"][0]["code"] == 0, "Expected one acknowledged CHANGE_CONFIG command (ui.proto action 6)"
                        assert report["commands"][0]["configuration"] == {"DefaultAction": "deny"}
                        result["persisted_daemon_configuration"] = {"DefaultAction": "deny"}
                    if args.feature == "firewall-policy":
                        from opensnitch.proto import ui_pb2 as proto
                        from google.protobuf.json_format import MessageToDict
                        payload=db.execute("SELECT firewall FROM node_info WHERE name='Synthetic local daemon'").fetchone()[0]
                        saved=proto.SysFirewall.FromString(payload)
                        expected=proto.SysFirewall(Enabled=False,Version=1)
                        chain=expected.SystemRules.add().Chains.add(Name="fixture-input",Table="filter",Family="inet",Type="filter",Hook="input",Policy="accept")
                        chain.Rules.add(UUID="fixture-system-rule",Enabled=True,Description="Synthetic system rule",Target="accept")
                        assert saved==expected,"Native disabling lost unedited firewall fields"
                        assert db.execute("SELECT length(fingerprint) FROM firewall_accepted WHERE node='Synthetic local daemon'").fetchone()==(32,),"Final disabled cache marker missing"
                        preparation=proto.SysFirewall();preparation.CopyFrom(expected);preparation.Enabled=True
                        commands=json.loads((evidence / "fixture-report.json").read_text())["commands"]
                        assert len(commands)==2 and all(c["type"]==5 and c["code"]==0 and not c["rules"] for c in commands),"Expected exactly two accepted RELOAD_FW_RULES commands"
                        assert [c["system_firewall"] for c in commands]==[MessageToDict(preparation,preserving_proto_field_name=True),MessageToDict(expected,preserving_proto_field_name=True)],"Native preparation/disable sequence or complete payload changed"
                        result["persisted_system_firewall"]={"enabled":False,"policy":"accept","rules":1,"accepted":True,"stages":2}
                    if args.feature == "firewall-forms":
                        from opensnitch.proto import ui_pb2 as proto
                        from google.protobuf.json_format import MessageToDict
                        payload=db.execute("SELECT firewall FROM node_info WHERE name='Synthetic local daemon'").fetchone()[0]
                        saved=proto.SysFirewall.FromString(payload)
                        expected=proto.SysFirewall(Enabled=True,Version=1)
                        chain=expected.SystemRules.add().Chains.add(Name="fixture-input-native",Table="filter",Family="inet",Type="filter",Hook="input",Policy="accept")
                        chain.Rules.add(UUID="fixture-system-rule",Enabled=True,Description="Synthetic system rule",Target="accept")
                        assert saved==expected,"Native chain form lost or defaulted unedited firewall fields"
                        assert db.execute("SELECT length(fingerprint) FROM firewall_accepted WHERE node='Synthetic local daemon'").fetchone()==(32,),"Native accepted marker was not persisted"
                        commands=json.loads((evidence / "fixture-report.json").read_text())["commands"]
                        assert len(commands)==1 and commands[0]["type"]==5 and commands[0]["code"]==0 and not commands[0]["rules"],"Expected one accepted complete system-firewall reload only"
                        assert commands[0]["system_firewall"]==MessageToDict(expected,preserving_proto_field_name=True)
                        result["persisted_system_firewall"]={"backend":"nftables","version":1,"chain":"fixture-input-native","rules":1,"accepted":True}
                    if args.feature == "history-preferences":
                        saved=db.execute("SELECT days,mib,cleanup_minutes,edited FROM history_preferences WHERE id=1").fetchone()
                        assert saved==(1,256,5,1), "Native preferences lost unedited values or failed to save one day"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "History preferences sent a daemon policy command"
                        result["persisted_retention"]={"days":1,"mib":256,"cleanup_minutes":5}
                    if args.feature == "live":
                        report=json.loads((evidence / "fixture-report.json").read_text())
                        commands=report["commands"]
                        assert [c["type"] for c in commands]==[13,14], "Native live lifecycle must send one start and one stop"
                        expected={"name":"sockets-monitor","data":{"interval":"5s","state":0,"proto":0,"family":0}}
                        assert all(c["task"]==expected for c in commands), "Native live task config changed unexpectedly"
                        assert all(not c["acknowledged"] for c in commands), "Fixture must not invent stop acknowledgement"
                        result["live_task_commands"] = [13,14]
                    if args.feature == "alerts":
                        rows=db.execute("SELECT node,severity,priority,action,summary FROM alerts ORDER BY id").fetchall()
                        assert len(rows)==6 and all(row[:4]==("Synthetic local daemon",1,2,1) for row in rows),"Native alert metadata was not persisted"
                        assert [row[4] for row in rows]==[f"Synthetic daemon alert {i:03}" for i in range(6)]
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"],"Alert browsing issued policy commands"
                        result["persisted_alerts"]=len(rows)
                    if args.feature == "interception-preference":
                        name,running=db.execute("SELECT name,running FROM node_pause").fetchone()
                        assert (name,running)==("Synthetic local daemon",1), "Native pause preference was not saved for the exact node"
                        assert db.execute("SELECT running FROM node_info WHERE name=?",(name,)).fetchone()[0]==1, "Saving pause preference changed the current daemon state"
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Saving pause preference emitted a daemon command"
                        result["persisted_pause_preference"]={"node":name,"remember":True,"last_confirmed_running":True,"policy_commands":0}
                    if args.feature == "subscription-inheritance":
                        with sqlite3.connect((evidence / "subscriptions/subscriptions.sqlite3").as_uri()+"?mode=ro",uri=True) as lists:
                            rows=lists.execute("SELECT config,checked,bytes,file FROM subscriptions").fetchall()
                            assert len(rows)==1 and rows[0][1:]==(0,0,None)
                            config=json.loads(rows[0][0])
                            assert config["name"]=="Native inherited limits"
                            for name,unit in (("interval","minutes"),("timeout","seconds"),("max_bytes","KB")):
                                assert config[name]=={"count":None,"unit":unit},"Native unit-only override was flattened or lost"
                            settings=json.loads(lists.execute("SELECT config FROM settings WHERE id=1").fetchone()[0])
                            assert settings["units"]=={"interval":"hours","timeout":"seconds","size":"MB"} and settings["automatic"] is False
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"]
                        result["unit_only_overrides"]={key:config[key] for key in ("interval","timeout","max_bytes")}
                        result["downloaded_bytes"]=0
                    if args.feature == "subscription-url":
                        report=json.loads((evidence / "fixture-report.json").read_text())
                        assert not report["commands"], "URL diagnostics sent a policy command"
                        with sqlite3.connect((evidence / "subscriptions/subscriptions.sqlite3").as_uri()+"?mode=ro",uri=True) as lists:
                            assert lists.execute("SELECT count(*) FROM subscriptions").fetchone()==(0,), "URL test saved a subscription"
                            assert lists.execute("SELECT count(*) FROM logs").fetchone()==(0,), "URL test changed subscription logs"
                        result["url_test"]={"unsupported_scheme_refused":True,"persisted_subscriptions":0,"subscription_logs":0,"policy_commands":0,"http_scope":"Unsupported scheme rejected before starting a request"}
                    if args.feature in ("window-state", "window-restore"):
                        assert not json.loads((evidence / "fixture-report.json").read_text())["commands"], "Window restoration issued a policy command"
                        state_file=evidence / "history.sqlite3.window.json"
                        result["saved_window_state_sha256"]=hashlib.sha256(state_file.read_bytes()).hexdigest()
                        result["persisted_window_state"]=json.loads(state_file.read_text())
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
                    if args.feature in ("tray", "prompts", "placement", "countdown", "checksums"):
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
