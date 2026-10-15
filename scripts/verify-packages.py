#!/usr/bin/env python3
"""Check only owned packages in disposable Ubuntu containers; no host daemon use."""
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
PACKAGES = ROOT / "target/packages"


def main():
    subprocess.run(["sha256sum", "--check", "SHA256SUMS"], cwd=PACKAGES, check=True)
    scripts = {
        "deb-and-archive": r'''set -eu
apt-get update
apt-get install -y /packages/rustleblower_*.deb
test -f /usr/share/applications/dev.rustleblower.Rustleblower.desktop
test -f /usr/share/doc/rustleblower/copyright
rustleblower --version | grep -Fx "rustleblower @VERSION@"
mkdir /archive
tar -xzf /packages/rustleblower-*-linux-x86_64.tar.gz -C /archive
/archive/rustleblower-*/rustleblower --version | grep -Fx "rustleblower @VERSION@"
set +e
env -u DISPLAY -u WAYLAND_DISPLAY rustleblower --socket /tmp/package-proof.sock --database /tmp/package-proof.sqlite3 2>/tmp/display-error
status=$?
set -e
test "$status" -eq 1
grep -F "Launch from a Wayland/X11 desktop" /tmp/display-error
test ! -e /tmp/package-proof.sock
''',
        "appimage": r'''set -eu
/packages/rustleblower-*-linux-x86_64.AppImage --appimage-extract-and-run --version | grep -Fx "rustleblower @VERSION@"
set +e
env -u DISPLAY -u WAYLAND_DISPLAY /packages/rustleblower-*-linux-x86_64.AppImage --appimage-extract-and-run --socket /tmp/package-proof.sock --database /tmp/package-proof.sqlite3 2>/tmp/display-error
status=$?
set -e
test "$status" -eq 1
grep -F "Launch from a Wayland/X11 desktop" /tmp/display-error
test ! -e /tmp/package-proof.sock
''',
    }
    for name, script in scripts.items():
        print(f"Checking {name}", flush=True)
        subprocess.run(["docker", "run", "--rm", "-v", f"{PACKAGES}:/packages:ro", "ubuntu:24.04", "bash", "-euc", script.replace("@VERSION@", VERSION)], check=True)
    print(json.dumps({"status": "passed", "version": VERSION, "checks": list(scripts)}))


if __name__ == "__main__":
    main()
