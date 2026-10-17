#!/usr/bin/env python3
"""FerryDock-style real GPU/input verification without taking desktop focus."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import tomllib
import uuid

from headless_suites import FEATURE_TESTS, log_timings

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--feature", choices=tuple(FEATURE_TESTS), default="all")
    args = parser.parse_args()
    evidence = (args.evidence or ROOT / ".verification/headless" /
                (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8])).resolve()
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    env = os.environ.copy()
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    env["RUSTLEBLOWER_HEADLESS_EVIDENCE"] = str(evidence)
    if args.feature in ("startup", "settings"):
        env["XDG_CONFIG_HOME"] = str(evidence / "xdg")
        env["XDG_CONFIG_DIRS"] = str(evidence / "system-xdg")
    if args.feature == "permission-icons":
        env["XDG_DATA_HOME"] = str(evidence / "desktop-data")
        env["XDG_DATA_DIRS"] = str(evidence / "empty-system-data")
    test = FEATURE_TESTS[args.feature]
    command = [sys.executable, str(ROOT / "scripts/cargo.py"), "test", "--lib", "--features", "headless-tests", "--locked", "--offline",
               "app::headless_tests::" + test, "--", "--exact", "--nocapture", "--test-threads=1"]
    if args.feature in ("subscriptions", "subscription-url", "subscription-inheritance", "updates", "reputation", "raw-downloads"):
        command = [sys.executable, str(ROOT / "scripts/owned_net.py"), "--evidence", str(evidence / "network.json"), "--", *command]
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    def locked_version(name):
        versions = [package["version"] for package in packages if package["name"] == name]
        if len(versions) != 1:
            raise RuntimeError(f"Expected one locked {name} package, found {versions}")
        return versions[0]
    result = {"status": "failed", "feature": args.feature, "evidence": str(evidence), "command": command,
              "engine": "gpui-fast", "engine_version": locked_version("gpui-fast"), "kit_version": locked_version("gpui-kit"),
              "cargo_lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest()}
    started = time.monotonic()
    with (evidence / "test.log").open("w") as log:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=log, start_new_session=True)
        try:
            code = process.wait(timeout=args.timeout)
            result["exit_code"] = code
            if code:
                raise RuntimeError("GPU/input flow failed; inspect test.log")
            flow = json.loads((evidence / "headless.json").read_text())
            if flow["status"] != "passed" or not flow["real_gpu"] or not flow["scoped_socket_removed"]:
                raise RuntimeError("Missing successful GPU flow/cleanup proof")
            if not flow["screenshots"] or not all((evidence / p).is_file() for p in flow["screenshots"]):
                raise RuntimeError("Missing screenshots")
            if args.feature in ("subscriptions", "subscription-url", "subscription-inheritance", "updates", "reputation", "raw-downloads"):
                network=json.loads((evidence / "network.json").read_text())
                if network["network_namespace"]==network["parent_network_namespace"] or not network["host_network_unchanged"]:
                    raise RuntimeError("Owned HTTP network isolation was not proven")
                result["network_isolation"]=str(evidence / "network.json")
            result.update(status="passed", flow=flow)
        except (Exception, KeyboardInterrupt) as error:
            result["error"] = str(error) or type(error).__name__
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            result.update(owned_process_exited=process.poll() is not None,
                          wall_seconds=round(time.monotonic() - started, 3))
            result.update(log_timings((evidence / "test.log").read_text(errors="replace")))
            (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("status", "feature", "evidence", "build_seconds", "test_seconds", "wall_seconds", "owned_process_exited")}, indent=2))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
