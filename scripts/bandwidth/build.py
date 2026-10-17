#!/usr/bin/env python3
"""Build and stage a local collector; --portable builds release assets without host BTF."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SOURCE = Path(__file__).resolve().parent
FUNCTIONS = {
    "tcp_sendmsg_locked": ["sk", "msg", "size"],
    "tcp_recvmsg": ["sk", "msg", "len", "flags"],
    "udp_sendmsg": ["sk", "msg", "len"],
    "udp_recvmsg": ["sk", "msg", "len", "flags"],
    "udpv6_sendmsg": ["sk", "msg", "len"],
    "udpv6_recvmsg": ["sk", "msg", "len", "flags"],
    "bpf_lsm_socket_sendmsg": ["sock", "msg", "size"],
    "bpf_lsm_socket_recvmsg": ["sock", "msg", "size", "flags"],
}
KFUNCS = ("bpf_get_task_exe_file", "bpf_path_d_path", "bpf_put_file")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--no-stage", action="store_true", help="Keep an isolated build without updating the source UI helper")
    parser.add_argument("--portable", action="store_true", help="Build CO-RE package assets without requiring build-host kernel features")
    args = parser.parse_args()
    for command in (*(("bpftool",) if not args.portable else ()), "clang", "cc", "pkg-config"):
        if not shutil.which(command):
            parser.error(f"Missing build dependency: {command}")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    output = (args.output or ROOT / ".verification/bandwidth" / f"build-{stamp}").resolve()
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    result = {"status": "failed", "kernel": platform.release(), "directory": str(output),
              "portable": args.portable, "loaded_bpf": False, "changed_daemon": False, "commands": []}
    with (output / "build.log").open("w") as log:
        def run(command, stdout=log):
            result["commands"].append(command)
            return subprocess.run(command, check=True, stdout=stdout, stderr=log, text=True)
        try:
            if args.portable:
                shutil.copyfile(SOURCE / "kernel-types.h", output / "vmlinux.h")
            else:
                raw = output / "kernel-btf.txt"
                with raw.open("w") as file:
                    run(["bpftool", "btf", "dump", "file", "/sys/kernel/btf/vmlinux", "format", "raw"], file)
                blocks = re.split(r"(?m)(?=^\[\d+\])", raw.read_text())
                by_id = {re.match(r"\[(\d+)\]", item)[1]: item for item in blocks if re.match(r"\[(\d+)\]", item)}
                def type_header(type_id):
                    header = by_id[type_id].splitlines()[0]
                    for _ in range(16):
                        if not re.search(r"\] (TYPEDEF|CONST|VOLATILE|RESTRICT|TYPE_TAG) ", header):
                            return header
                        type_id = re.search(r"type_id=(\d+)", header)[1]
                        header = by_id[type_id].splitlines()[0]
                    raise RuntimeError("Excessively nested BTF type")

                def argument_type(type_id, expected):
                    header = type_header(type_id)
                    if expected in ("sk", "sock", "msg"):
                        struct = {"sk": "sock", "sock": "socket", "msg": "msghdr"}[expected]
                        if not re.search(r"\] PTR ", header):
                            return False
                        pointed = type_header(re.search(r"type_id=(\d+)", header)[1])
                        return f"STRUCT '{struct}' " in pointed
                    if expected in ("flags", "return", "size32"):
                        return "INT " in header and "size=4 " in header and "nr_bits=32 " in header and "encoding=SIGNED" in header
                    return "INT " in header and "size=8 " in header and "nr_bits=64 " in header and "encoding=(none)" in header

                signatures = {}
                for name, expected in FUNCTIONS.items():
                    function = next((item for item in blocks if re.search(rf"FUNC '{name}' ", item)), None)
                    if function is None:
                        raise RuntimeError(f"Kernel BTF lacks {name}; no partial collector will be built")
                    proto = by_id[re.search(r"type_id=(\d+)", function)[1]]
                    names = re.findall(r"^\s+'([^']+)' type_id=", proto, re.M)
                    if names != expected:
                        raise RuntimeError(f"Unsupported {name} signature: {names}; expected {expected}")
                    arguments = re.findall(r"^\s+'([^']+)' type_id=(\d+)", proto, re.M)
                    returned = re.search(r"ret_type_id=(\d+)", proto)[1]
                    if not argument_type(returned, "return") or not all(
                            argument_type(t, "size32" if name.startswith("bpf_lsm_") and n == "size" else n)
                            for n, t in arguments):
                        raise RuntimeError(f"Unsupported {name} argument/return types: {proto.strip()}")
                    signatures[name] = proto.strip()
                result["signatures"] = signatures
                for name in KFUNCS:
                    if not any(re.search(rf"FUNC '{name}' ", item) for item in blocks):
                        raise RuntimeError(f"Kernel BTF lacks {name}; executable capture requires BPF LSM filesystem kfuncs")
                result["required_kfuncs"] = list(KFUNCS)
                with (output / "vmlinux.h").open("w") as file:
                    run(["bpftool", "btf", "dump", "file", "/sys/kernel/btf/vmlinux", "format", "c"], file)
            run(["clang", "-g", "-O2", "-target", "bpf", "-Wall", "-Werror",
                 "-I", str(output), "-I", str(SOURCE), "-c", str(SOURCE / "accounting.bpf.c"),
                 "-o", str(output / "accounting.bpf.o")])
            flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "libbpf"], text=True).split()
            run(["cc", "-std=c11", "-g", "-O2", "-Wall", "-Wextra", "-Werror",
                 str(SOURCE / "collector.c"), "-o", str(output / "collector"),
                 "-Wl,--disable-new-dtags,-rpath,$ORIGIN/lib", *flags])
            if args.portable:
                libraries = output / "lib"
                libraries.mkdir()
                result["libraries"] = {}
                linked = subprocess.check_output(["ldd", str(output / "collector")], text=True)
                if "not found" in linked:
                    raise RuntimeError("Unresolved collector shared library")
                for name, source in re.findall(r"^\s*(\S+) => (/\S+) ", linked, re.M):
                    if name == "libc.so.6" or name.startswith(("/", "ld-linux")):
                        continue
                    if not re.fullmatch(r"lib[a-zA-Z0-9_-]+\.so[.0-9]*", name):
                        raise RuntimeError(f"Unexpected collector library: {name}")
                    shutil.copyfile(source, libraries / name)
                    result["libraries"][name] = source
            run(["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
                 str(SOURCE / "test_collector.c"), "-o", str(output / "test-collector"), *flags])
            run([str(output / "test-collector")])
            result["userspace_path_checks"] = "passed"
            result["sha256"] = {name: hashlib.sha256((output / name).read_bytes()).hexdigest()
                                for name in ("collector", "accounting.bpf.o")}
            if args.portable:
                result["sha256"].update({f"lib/{name}": hashlib.sha256((output / "lib" / name).read_bytes()).hexdigest()
                                         for name in result["libraries"]})
            result["source_sha256"] = {name: hashlib.sha256((SOURCE / name).read_bytes()).hexdigest()
                                       for name in ("accounting.h", "accounting.bpf.c", "collector.c", "test_collector.c", "build.py", "kernel-types.h")}
            if not args.no_stage:
                # Resolve this link once in the UI worker, so helper/object always
                # come from one immutable evidence build, even during a rebuild.
                target = ROOT / "target"
                target.mkdir(exist_ok=True)
                staged = target / "bandwidth"
                temporary = target / f".bandwidth-{stamp}"
                try:
                    temporary.symlink_to(output, target_is_directory=True)
                    temporary.replace(staged)
                finally:
                    temporary.unlink(missing_ok=True)
                result["staged"] = str(staged)
            result["status"] = "passed"
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            result["error"] = str(error)
        finally:
            (output / "build.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ("status", "directory", "loaded_bpf")}))
    if result["status"] != "passed":
        raise SystemExit(f"Build failed: {result.get('error')}; inspect {output / 'build.log'}")


if __name__ == "__main__":
    main()
