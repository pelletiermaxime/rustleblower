#!/usr/bin/env python3
"""Compare Rust range fixtures with unmodified pinned daemon function bodies.

Only an isolated Go package is executed. No daemon, RPC, firewall or network
package is linked; extracted upstream code stays in ignored verification output.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

from importlib.util import module_from_spec, spec_from_file_location

PIN = "a1353848ba1b660320e90cefea782c3fba272c00"
ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--git-dir", type=Path, default=Path("/home/maximep/.cache/paru/clone/opensnitch-git/opensnitch"))
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    raw = subprocess.check_output(["git", f"--git-dir={args.git_dir}", "show", f"{PIN}:daemon/rule/operator.go"])
    fixtures = args.fixtures.resolve() / "grammar.json"
    cases = json.loads(fixtures.read_text())
    assert len(cases) == 21
    spec = spec_from_file_location("list_matching", ROOT / "scripts/verify-list-matching.py")
    helper = module_from_spec(spec)
    spec.loader.exec_module(helper)
    methods = "\n".join(helper.function(raw.decode(), name) for name in ["compileRange", "rangeCmp"])
    work = evidence / "go"
    work.mkdir()
    (work / "go.mod").write_text("module rustleblower-range-proof\n\ngo 1.25.0\n")
    (work / "operator.go").write_text('package proof\nimport("fmt";"strconv";"strings")\ntype Operator struct{Data string;rangeMin,rangeMax uint64}\n' + methods)
    (work / "operator_test.go").write_text(GO_TEST)
    with tempfile.TemporaryDirectory(prefix="rustleblower-range-go-") as cache:
        env = dict(os.environ, GOCACHE=cache, GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local", GOWORK="off", FIXTURES=str(fixtures))
        with (evidence / "go-test.log").open("w") as log:
            subprocess.run(["go", "test", "-count=1", "-v", "."], cwd=work, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    report = {"status":"passed", "commit":PIN, "source_sha256":hashlib.sha256(raw).hexdigest(),
              "fixtures_sha256":hashlib.sha256(fixtures.read_bytes()).hexdigest(), "grammar_cases":len(cases),
              "methods":["compileRange","rangeCmp"], "real_daemon_started":False,
              "proof_boundary":"Unmodified pinned compileRange/rangeCmp bodies compared with production Rust parser fixtures, including uint64 limits and inclusive boundaries. No kernel enforcement or live-daemon capability proof."}
    (evidence / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


GO_TEST = r'''package proof
import("encoding/json";"os";"strconv";"testing")
type Case struct{Data string;Bounds []string}
func TestRustGrammarAndPinnedGoSemantics(t *testing.T){
 raw,err:=os.ReadFile(os.Getenv("FIXTURES"));if err!=nil{t.Fatal(err)}
 var cases []Case;if err=json.Unmarshal(raw,&cases);err!=nil{t.Fatal(err)}
 for _,c:=range cases{
  op:=Operator{Data:c.Data};err=op.compileRange()
  if (err==nil)!=(len(c.Bounds)==2){t.Errorf("%q error=%v bounds=%v",c.Data,err,c.Bounds);continue}
  if err!=nil{continue}
  lo,e:=strconv.ParseUint(c.Bounds[0],10,64);if e!=nil{t.Fatal(e)}
  hi,e:=strconv.ParseUint(c.Bounds[1],10,64);if e!=nil{t.Fatal(e)}
  if op.rangeMin!=lo||op.rangeMax!=hi{t.Fatalf("%q bounds differ",c.Data)}
  values:=[]uint64{0,1,80,81,443,3999,4000,4005,4010,4011,7999,8000,8080,8081,65535,65536,^uint64(0),lo,hi}
  if lo>0{values=append(values,lo-1)};if hi<^uint64(0){values=append(values,hi+1)}
  for _,n:=range values{if got:=op.rangeCmp(strconv.FormatUint(n,10));got!=(lo<=n&&n<=hi){t.Errorf("%q value %d mismatch",c.Data,n)}}
 }
}
'''

if __name__ == "__main__":
    main()
