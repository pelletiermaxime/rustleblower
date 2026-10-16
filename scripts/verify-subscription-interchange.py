#!/usr/bin/env python3
"""Compare real Rust configuration fixtures with the pinned upstream Python codec.

Only pure models/helpers are loaded. No plugin, UI, downloader or daemon is run.
All retrieved source and results remain in a fresh ignored evidence directory.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import types

ROOT = Path(__file__).resolve().parents[1]
PIN = "a1353848ba1b660320e90cefea782c3fba272c00"
PREFIX = "opensnitch.plugins.list_subscriptions"
SOURCES = ("_utils", "models.global_defaults", "models.subscriptions", "models.config", "models.action")


def canonical(plugin, helpers):
    defaults = plugin.defaults
    settings = {
        "interval": helpers.to_seconds(defaults.interval, defaults.interval_units, 86400),
        "timeout": helpers.to_seconds(defaults.timeout, defaults.timeout_units, 60),
        "max_bytes": helpers.to_max_bytes(defaults.max_size, defaults.max_size_units, 20 * 1024 * 1024),
        "user_agent": defaults.user_agent,
    }
    subscriptions = []
    for spec in plugin.subscriptions:
        subscriptions.append({
            "name": spec.name, "url": spec.url, "filename": spec.filename,
            "format": spec.format, "enabled": spec.enabled, "groups": list(spec.groups),
            "interval": spec.interval_seconds, "timeout": spec.timeout_seconds,
            "max_bytes": spec.max_bytes,
        })
    return settings, sorted(subscriptions, key=lambda item: item["name"])


def rust_canonical(document):
    settings = {key: document["settings"][key] for key in ("interval", "timeout", "max_bytes", "user_agent")}
    subscriptions = []
    for spec in document["subscriptions"]:
        spec = dict(spec)
        for key in ("interval", "timeout", "max_bytes"):
            if spec[key] is None:
                spec[key] = settings[key]
            elif isinstance(spec[key],dict):
                factors={"seconds":1,"minutes":60,"hours":3600,"days":86400,"weeks":604800,"bytes":1,"KB":1024,"MB":1048576,"GB":1073741824}
                global_key={"interval":"interval","timeout":"timeout","max_bytes":"size"}[key]
                units=document["settings"].get("units") or {"interval":"seconds","timeout":"seconds","size":"bytes"}
                inherited=units[global_key]
                count=spec[key]["count"]
                unit=spec[key]["unit"] or inherited
                spec[key]=(settings[key]//factors[inherited] if count is None else count)*factors[unit]
        subscriptions.append(spec)
    return settings, sorted(subscriptions, key=lambda item: item["name"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--inheritance-cases", type=Path)
    parser.add_argument("--git-dir", default="/home/maximep/.cache/paru/clone/opensnitch-git/opensnitch")
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    report = {"status": "failed", "pin": PIN, "sources": {}, "checks": []}
    try:
        for package in ("opensnitch", "opensnitch.utils", "opensnitch.plugins", PREFIX, PREFIX + ".models"):
            module = types.ModuleType(package)
            module.__path__ = []
            sys.modules[package] = module
        xdg = types.ModuleType("opensnitch.utils.xdg")
        xdg.xdg_config_home = str(evidence / "owned-config")
        sys.modules[xdg.__name__] = xdg
        modules = {}
        for relative in SOURCES:
            source = "ui/opensnitch/plugins/list_subscriptions/" + relative.replace(".", "/") + ".py"
            data = subprocess.check_output(["git", "--git-dir=" + args.git_dir, "show", PIN + ":" + source], cwd=ROOT)
            path = evidence / "sources" / source
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            report["sources"][source] = hashlib.sha256(data).hexdigest()
            name = PREFIX + "." + relative
            spec = importlib.util.spec_from_file_location(name, path)
            module = importlib.util.module_from_spec(spec)
            sys.modules[name] = module
            spec.loader.exec_module(module)
            modules[relative] = module
        helpers = modules["_utils"]
        codec = modules["models.config"].PluginConfig
        fixtures = args.fixtures.resolve()
        cases = json.loads((fixtures / "reference-cases.json").read_text())
        for case in cases:
            config = case["source"]["actions"]["list_subscriptions"]["config"]
            expected = canonical(codec.from_dict(config), helpers)
            actual = rust_canonical(case["actual"])
            assert actual == expected, f"{case['name']}: Rust {actual!r} differs from upstream {expected!r}"
            report["checks"].append(case["name"])
        imported = json.loads((fixtures / "native-imported.json").read_text())
        exported = json.loads((fixtures / "exported-action.json").read_text())
        config = exported["actions"]["list_subscriptions"]["config"]
        assert canonical(codec.from_dict(config), helpers) == rust_canonical(imported), "Production exported action changed effective subscription configuration"
        action = modules["models.action"].MutableActionConfig.from_action_dict(exported)
        assert action.action_name == "ownedAction" and action.enabled is False
        assert len(action.plugin.subscriptions) == len(imported["subscriptions"])
        report["checks"].append("production action export accepted by pinned model")
        duplicate_native=fixtures / "duplicate-native.json"
        duplicate_action=fixtures / "duplicate-action.json"
        if duplicate_native.exists() or duplicate_action.exists():
            backup=json.loads(duplicate_native.read_text())
            portable=json.loads(duplicate_action.read_text())
            config=portable["actions"]["list_subscriptions"]["config"]
            native_settings,native_rows=rust_canonical(backup)
            portable_settings,portable_rows=canonical(codec.from_dict(config,invalidate_duplicates=True),helpers)
            assert native_settings==portable_settings and len(native_rows)==len(portable_rows)==2
            assert len({row["filename"] for row in portable_rows})==2
            for original,converted in zip(native_rows,portable_rows):
                assert {k:v for k,v in original.items() if k!="filename"}=={k:v for k,v in converted.items() if k!="filename"}
            action_model=modules["models.action"].MutableActionConfig.from_action_dict(portable)
            assert len(action_model.plugin.normalize_subscriptions(invalidate_duplicates=True))==2
            report["checks"].append("duplicate names export as distinct valid OpenSnitch filenames")
            rows=backup["opensnitch"]["actions"]["list_subscriptions"]["config"]["subscriptions"]
            assert [row["tag"] for row in rows]==["first","second"]
            assert backup["subscriptions"][0]["filename"]==backup["subscriptions"][1]["filename"]
            report["checks"].append("native duplicate backup preserves filenames and ordered opaque fields")
        if args.inheritance_cases:
            data=args.inheritance_cases.read_bytes()
            assert len(data)<=2*1024*1024
            cases=json.loads(data)
            assert 1<=len(cases)<=128
            comparisons=0
            for index,case in enumerate(cases):
                config=case["action"]["actions"]["list_subscriptions"]["config"]
                _,actual=canonical(codec.from_dict(config),helpers)
                actual=[{key:row[key] for key in ("name","interval","timeout","max_bytes")} for row in actual]
                assert actual==sorted(case["values"],key=lambda row:row["name"]),f"Inheritance phase {index} differs from pinned source"
                comparisons+=len(actual)
                report["checks"].append(f"independent inheritance phase {index}")
            report["inheritance_values"]=comparisons
            report["inheritance_cases_sha256"]=hashlib.sha256(data).hexdigest()
        report.update(status="passed", fixtures_sha256={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in fixtures.glob("*.json")},
                      proof_boundary="Pure pinned Python models and helpers against actual Rust decoder/worker exports. No Qt/plugin runtime, daemon, networking or policy enforcement.")
    except Exception as error:
        report["error"] = str(error)
    finally:
        (evidence / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("status", "checks")}, indent=2))
    if report["status"] != "passed":
        print(report.get("error", "Unknown verification failure"), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
