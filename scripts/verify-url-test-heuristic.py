#!/usr/bin/env python3
"""Compare actual Rust URL-test samples to the pinned pure OpenSnitch heuristic."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import subprocess

PIN = "a1353848ba1b660320e90cefea782c3fba272c00"
SOURCE = "ui/opensnitch/plugins/list_subscriptions/_utils.py"

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--git-dir", default="/home/maximep/.cache/paru/clone/opensnitch-git/opensnitch")
    args = parser.parse_args()
    raw = subprocess.check_output(["git", "--git-dir=" + args.git_dir, "show", PIN + ":" + SOURCE])
    tree = ast.parse(raw)
    node = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "is_hosts_file_like")
    namespace = {}
    exec(compile(ast.Module(body=[node], type_ignores=[]), SOURCE, "exec"), namespace)
    data = args.fixtures.read_bytes()
    assert len(data) <= 1024 * 1024
    cases = json.loads(data)
    assert 1 <= len(cases) <= 256
    for case in cases:
        assert len(case["lines"]) <= 200
        expected = namespace["is_hosts_file_like"](case["lines"])
        assert expected == case["hosts_like"], case
    args.evidence.mkdir(parents=True, exist_ok=False)
    (args.evidence / "upstream-utils.py").write_bytes(raw)
    result = {"status": "passed", "cases": len(cases), "pin": PIN, "source": SOURCE,
              "source_sha256": hashlib.sha256(raw).hexdigest(),
              "fixtures_sha256": hashlib.sha256(data).hexdigest(),
              "boundary": "Actual Rust results compared to the single unmodified pinned pure helper; no plugin, Qt, network or daemon runtime executed."}
    (args.evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))

if __name__ == "__main__":
    main()
