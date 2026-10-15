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
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--feature", choices=("all", "popup-preferences", "countdown", "checksums", "history-tools", "statistics", "nodes", "daemon-preferences"), default="all")
    args = parser.parse_args()
    evidence = (args.evidence or ROOT / ".verification/headless" /
                (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8])).resolve()
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    env = os.environ.copy()
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    env["RUSTLEBLOWER_HEADLESS_EVIDENCE"] = str(evidence)
    test = {"all": "headless_gpu_user_flow", "popup-preferences": "headless_preferences_controls", "countdown": "headless_countdown_review", "checksums": "headless_checksum_review", "history-tools": "headless_history_tools", "statistics": "headless_statistics", "nodes": "headless_nodes", "daemon-preferences": "headless_daemon_preferences"}[args.feature]
    command = [sys.executable, str(ROOT / "scripts/cargo.py"), "test", "--lib", "--features", "headless-tests", "--locked", "--offline",
               "app::headless_tests::" + test, "--", "--nocapture", "--test-threads=1"]
    result = {"status": "failed", "evidence": str(evidence), "command": command,
              "engine": "gpui-fast", "engine_version": "0.1.7", "kit_version": "0.7.3",
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
            (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("status", "evidence", "wall_seconds", "owned_process_exited")}, indent=2))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
