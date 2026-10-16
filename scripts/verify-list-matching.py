#!/usr/bin/env python3
"""Check production-normalized fixtures with pinned upstream Go list readers.

Upstream source is extracted into ignored evidence only. The harness links no
interception/backend packages, never starts a daemon and uses only local files.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

PIN = "a1353848ba1b660320e90cefea782c3fba272c00"
ROOT = Path(__file__).resolve().parents[1]


def function(source, name):
    match = re.search(r"^func (?:\([^\n]+\) )?" + re.escape(name) + r"\(", source, re.M)
    if not match:
        raise RuntimeError(f"Pinned function {name} missing")
    end = source.find("\nfunc ", match.end())
    return source[match.start():end if end >= 0 else len(source)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--git-dir", type=Path, default=Path("/home/maximep/.cache/paru/clone/opensnitch-git/opensnitch"))
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    sources = {}

    def source(name):
        raw = subprocess.check_output(["git", f"--git-dir={args.git_dir}", "show", f"{PIN}:{name}"])
        sources[name] = hashlib.sha256(raw).hexdigest()
        return raw.decode()

    fixture_dir = evidence / "normalized"
    environment = dict(os.environ, RUSTLEBLOWER_LIST_FIXTURE_EVIDENCE=str(fixture_dir))
    with (evidence / "production-fixtures.log").open("w") as log:
        subprocess.run(["python", str(ROOT / "scripts/owned_net.py"), "--evidence", str(evidence / "network.json"), "--", "python", str(ROOT / "scripts/cargo.py"), "test", "--no-default-features", "--test", "subscriptions", "--locked", "--offline", "canonical_publications_keep_stable_format_directories_and_all_retained_bytes", "--", "--exact"], cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert sorted(p.stem for p in fixture_dir.glob("*.txt")) == ["domains", "hosts", "ips", "md5", "nets", "regexp"]
    work = evidence / "go"
    (work / "rule").mkdir(parents=True)
    (work / "core").mkdir()
    (work / "log").mkdir()
    (work / "go.mod").write_text("module github.com/evilsocket/opensnitch/daemon\n\ngo 1.25.0\n")
    lists = source("daemon/rule/operator_lists.go")
    operator = source("daemon/rule/operator.go")
    core = source("daemon/core/core.go")
    (work / "rule/lists.go").write_text(lists)
    prelude = operator[operator.index("// Type is"):operator.index("// NewOperator")]
    names = ["matchListsCmp", "domainsListsCmp", "simpleListsCmp", "netListsCmp", "ipListsCmp", "reListCmp"]
    methods = "\n".join(function(operator, name) for name in names)
    (work / "rule/operator.go").write_text('package rule\nimport("net";"regexp";"strings";"sync";"sync/atomic";"github.com/evilsocket/opensnitch/daemon/log")\n' + prelude + methods)
    (work / "core/core.go").write_text('package core\nimport("fmt";"os";"strings";"time")\nconst defaultTrimSet="\\r\\n\\t "\n' + function(core, "Trim") + "\n" + function(core, "GetFileModTime"))
    (work / "log/log.go").write_text('package log\nfunc Debug(string,...interface{}){}\nfunc Info(string,...interface{}){}\nfunc Warning(string,...interface{}){}\nfunc Red(s string)string{return s}\n')
    (work / "rule/normalized_test.go").write_text(GO_TEST)
    with tempfile.TemporaryDirectory(prefix="rustleblower-go-cache-") as cache:
        environment = dict(os.environ, GOCACHE=cache, GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local", GOWORK="off", FIXTURES=str(fixture_dir))
        with (evidence / "go-test.log").open("w") as log:
            subprocess.run(["go", "test", "-count=1", "-v", "./rule"], cwd=work, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)
    report = {"status": "passed", "commit": PIN, "source_sha256": sources,
              "fixture_sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in fixture_dir.glob("*.txt")},
              "matching_methods": names, "real_daemon_started": False,
              "proof_boundary": "Production normalized downloads read by unmodified pinned Go list reader/matcher bodies. Logging is a no-op; no interception or RPC packages are linked. Includes owned-file monitor reload. No kernel enforcement proof."}
    (evidence / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


GO_TEST = r'''package rule
import("net";"os";"path/filepath";"testing";"time")
func fixture(t *testing.T, name string, operand Operand)*Operator{
 t.Helper();directory:=t.TempDir();data,err:=os.ReadFile(filepath.Join(os.Getenv("FIXTURES"),name+".txt"));if err!=nil{t.Fatal(err)}
 if err=os.WriteFile(filepath.Join(directory,"list.txt"),data,0600);err!=nil{t.Fatal(err)}
 op:=&Operator{Operand:operand,Data:directory};if err=op.readLists();err!=nil{t.Fatal(err)};return op
}
func TestProductionDomains(t *testing.T){
 op:=fixture(t,"domains",OpDomainsLists)
 for host,want:=range map[string]bool{"exact.example":true,"EXACT.EXAMPLE":true,"wild.example":false,"one.wild.example":true,"one.two.wild.example":true,"leading.example":false,"x.leading.example":true,"api3.glob.example":true,"api33.glob.example":false,"apia.glob.example":false,"x.api3.glob.example":false,"other.example":false}{if got:=op.domainsListsCmp(host);got!=want{t.Errorf("%s: %v want %v",host,got,want)}}
}
func TestProductionHostsAliases(t *testing.T){op:=fixture(t,"hosts",OpDomainsLists);for host,want:=range map[string]bool{"first.example":true,"alias.example":true,"localhost":false,"unrelated.example":false}{if op.domainsListsCmp(host)!=want{t.Error(host)}}}
func TestProductionMixedIPAndCIDR(t *testing.T){for _,name:=range []string{"ips","nets"}{operand:=OpIPLists;if name=="nets"{operand=OpNetLists};op:=fixture(t,name,operand);for ip,want:=range map[string]bool{"192.0.2.1":true,"192.0.2.2":false,"198.51.100.42":true,"198.51.101.1":false,"2001:db8:abcd::1":true,"2001:db9::1":false}{got:=op.ipListsCmp(net.ParseIP(ip));if name=="nets"{got=op.netListsCmp(net.ParseIP(ip))};if got!=want{t.Errorf("%s %s: %v",name,ip,got)}}}}
func TestProductionMD5(t *testing.T){op:=fixture(t,"md5",OpHashMD5Lists);if !op.simpleListsCmp("abcdef0123456789abcdef0123456789")||op.simpleListsCmp("00000000000000000000000000000000"){t.Fatal("MD5 mismatch")}}
func TestProductionRegexp(t *testing.T){op:=fixture(t,"regexp",OpDomainsRegexpLists);if !op.reListCmp("regex123.example")||op.reListCmp("regexa.example")||op.reListCmp("other-regex123.example"){t.Fatal("Go regexp mismatch")}}
func TestProductionMonitorAtomicReplacement(t *testing.T){
 op:=fixture(t,"hosts",OpDomainsLists);op.loadLists();defer op.StopMonitoringLists()
 // Establish the first monitor snapshot before replacing the file.
 time.Sleep(100*time.Millisecond)
 path:=filepath.Join(op.Data,"list.txt");tmp:=filepath.Join(op.Data,".incoming")
 if err:=os.WriteFile(tmp,[]byte("0.0.0.0 replacement.example\n"),0600);err!=nil{t.Fatal(err)}
 if err:=os.Rename(tmp,path);err!=nil{t.Fatal(err)}
 deadline:=time.Now().Add(6*time.Second)
 for time.Now().Before(deadline){if op.domainsListsCmp("replacement.example")&&!op.domainsListsCmp("first.example"){return};time.Sleep(20*time.Millisecond)}
 t.Fatal("Pinned monitor did not observe atomic replacement")
}
func TestProductionMonitorEqualCountMembershipReplacement(t *testing.T){
 op:=fixture(t,"hosts",OpDomainsLists)
 marker:=filepath.Join(op.Data,"0-refresh.txt")
 if err:=os.WriteFile(marker,[]byte("# Rustleblower collection generation\n"),0600);err!=nil{t.Fatal(err)}
 op.loadLists();defer op.StopMonitoringLists();time.Sleep(100*time.Millisecond)
 before,err:=os.Stat(marker);if err!=nil{t.Fatal(err)}
 if err=os.Remove(filepath.Join(op.Data,"list.txt"));err!=nil{t.Fatal(err)}
 if err=os.WriteFile(filepath.Join(op.Data,"another-id.txt"),[]byte("0.0.0.0 membership-replacement.example\n"),0600);err!=nil{t.Fatal(err)}
 // Exactly the same number of visible files; the stable marker must change.
 if err=os.Chtimes(marker,before.ModTime().Add(time.Second),before.ModTime().Add(time.Second));err!=nil{t.Fatal(err)}
 deadline:=time.Now().Add(6*time.Second)
 for time.Now().Before(deadline){if op.domainsListsCmp("membership-replacement.example")&&!op.domainsListsCmp("first.example"){return};time.Sleep(20*time.Millisecond)}
 t.Fatal("Pinned monitor missed equal-count membership replacement")
}

'''


if __name__ == "__main__":
    main()
