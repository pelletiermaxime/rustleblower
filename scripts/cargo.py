#!/usr/bin/env python3
"""Prepare local dependency patches, then forward all arguments to Cargo."""
import os
import subprocess
import sys
import tarfile
from prepare_dependencies import ROOT, prepare


def main():
    try:
        prepare(offline="--offline" in sys.argv[1:] or "--frozen" in sys.argv[1:])
    except (OSError, RuntimeError, subprocess.CalledProcessError, tarfile.TarError) as error:
        sys.exit(f"Dependency preparation failed: {error}")
    os.chdir(ROOT)
    os.execvp("cargo", ["cargo", *sys.argv[1:]])


if __name__ == "__main__":
    main()
