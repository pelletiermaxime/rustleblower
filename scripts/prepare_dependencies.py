#!/usr/bin/env python3
"""Prepare the pinned OpenSnitch transport patch without vendoring upstream source."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.4.20"
PACKAGE = f"h2-{VERSION}"
ARCHIVE_SHA256 = "7d29020232d6aa3fb1daca64c1127cf662cf97f254ae16c18c05b8ab635fc118"
PATCH = ROOT / "patches/h2-opensnitch.patch"
DEPS = ROOT / ".deps"
DESTINATION = DEPS / PACKAGE
PATCHED_FILES = ("Cargo.toml", "src/server.rs")


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fingerprint():
    return {"archive_sha256": ARCHIVE_SHA256, "patch_sha256": sha256(PATCH)}


def is_prepared():
    try:
        stamp = json.loads((DESTINATION / ".rustleblower-patch.json").read_text())
        return (stamp["inputs"] == fingerprint()
                and stamp["files"] == {name: sha256(DESTINATION / name) for name in PATCHED_FILES}
                and (DESTINATION / "src/lib.rs").is_file())
    except (OSError, ValueError, KeyError):
        return False


def archive_path(offline):
    cached = DEPS / "cache" / f"{PACKAGE}.crate"
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).expanduser()
    candidates = [cached, *sorted((cargo_home / "registry/cache").glob(f"*/{PACKAGE}.crate"))]
    for candidate in candidates:
        if candidate.is_file() and sha256(candidate) == ARCHIVE_SHA256:
            return candidate
    if offline:
        raise RuntimeError(f"{PACKAGE} is not cached; run scripts/prepare_dependencies.py once online")
    cached.parent.mkdir(parents=True, exist_ok=True)
    url = f"https://static.crates.io/crates/h2/{PACKAGE}.crate"
    print(f"Downloading {PACKAGE}", file=sys.stderr, flush=True)
    with urllib.request.urlopen(url, timeout=30) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != ARCHIVE_SHA256:
        raise RuntimeError(f"Checksum mismatch for {url}; dependency was not prepared")
    cached.write_bytes(data)
    return cached


def prepare(offline=False):
    DEPS.mkdir(exist_ok=True)
    # A shared source directory must not be replaced while another preparation
    # is still extracting it. Normal repeated builds only read the stamp.
    with (DEPS / ".prepare.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if is_prepared():
            return
        if shutil.which("patch") is None:
            raise RuntimeError("The 'patch' command is required to prepare the transport dependency")
        archive = archive_path(offline)
        with tempfile.TemporaryDirectory(prefix=".prepare-", dir=DEPS) as scratch:
            stage = Path(scratch)
            with tarfile.open(archive) as source:
                source.extractall(stage, filter="data")
            package = stage / PACKAGE
            subprocess.run(["patch", "--batch", "--fuzz=0", "-p0", "-i", str(PATCH)],
                           cwd=package, check=True, stdout=sys.stderr)
            stamp = {"inputs": fingerprint(), "files": {name: sha256(package / name) for name in PATCHED_FILES}}
            (package / ".rustleblower-patch.json").write_text(json.dumps(stamp, indent=2) + "\n")
            if DESTINATION.exists():
                shutil.rmtree(DESTINATION)
            package.rename(DESTINATION)
        print(f"Prepared {PACKAGE} with the OpenSnitch compatibility patch", file=sys.stderr, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true", help="Use only a verified local crate archive")
    args = parser.parse_args()
    try:
        prepare(offline=args.offline)
    except (OSError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.exit(1, f"Dependency preparation failed: {error}\n")


if __name__ == "__main__":
    main()
