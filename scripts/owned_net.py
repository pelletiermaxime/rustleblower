#!/usr/bin/env python3
"""Run owned loopback fixtures in a private user/network namespace."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--parent", help=argparse.SUPPRESS)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("A fixture command is required")
    if not args.inside:
        parent = os.readlink("/proc/self/ns/net")
        return subprocess.call(["unshare", "--user", "--map-root-user", "--net",
            sys.executable, str(Path(__file__).resolve()), "--inside", "--parent", parent,
            "--evidence", str(args.evidence.resolve()), "--", *command])
    current = os.readlink("/proc/self/ns/net")
    uid_map = Path("/proc/self/uid_map").read_text().strip().splitlines()
    mapping = [int(v) for v in uid_map[0].split()] if len(uid_map) == 1 else []
    if not args.parent or current == args.parent or len(mapping) != 3 or mapping[0] != 0 or mapping[1] == 0 or mapping[2] != 1:
        raise RuntimeError("Refusing fixture setup outside an owned mapped-user network namespace")
    links = json.loads(subprocess.check_output(["ip", "-j", "link", "show"]))
    if [link["ifname"] for link in links] != ["lo"]:
        raise RuntimeError("The fixture namespace contains interfaces other than its owned loopback")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    args.evidence.write_text(json.dumps({"status": "ready", "parent_network_namespace": args.parent,
        "network_namespace": current, "uid_map": uid_map,
        "interfaces": json.loads(subprocess.check_output(["ip", "-j", "addr", "show"])),
        "host_network_unchanged": True, "command": command}, indent=2) + "\n")
    return subprocess.call(command)


if __name__ == "__main__":
    sys.exit(main())
