#!/usr/bin/env python3
"""Owned loopback workloads in a private network namespace; no host policy changes."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import selectors
import socket
import subprocess
import sys
import time
import threading
import traceback

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(__file__).resolve()
MAX_LINE = 2 * 1024 * 1024
CASES = ("tcp4", "tcp6", "udp4", "udp6", "udp-mapped", "tcp-peek", "udp-peek-trunc", "empty")


def workload(case):
    """One process owns both endpoints, so exact sent/received sums are known."""
    if case == "empty":
        return {"sent": 0, "received": 0}
    tcp = case.startswith("tcp")
    family = socket.AF_INET6 if case.endswith("6") else socket.AF_INET
    address = "::1" if family == socket.AF_INET6 else "127.0.0.1"
    kind = socket.SOCK_STREAM if tcp else socket.SOCK_DGRAM
    amount, ack = (131071 if tcp else 1207), b"acknowledged!"
    with socket.socket(family, kind) as server:
        server.settimeout(3)
        server.bind((address, 0))
        if tcp:
            server.listen(1)
        client_family = socket.AF_INET6 if case == "udp-mapped" else family
        with socket.socket(client_family, kind) as client:
            client.settimeout(3)
            destination = ("::ffff:127.0.0.1", server.getsockname()[1]) if case == "udp-mapped" else server.getsockname()
            client.connect(destination)
            if tcp:
                peer, _ = server.accept()
                with peer:
                    peer.settimeout(3)
                    client.sendall(b"x" * amount)
                    if case == "tcp-peek":
                        assert len(peer.recv(128, socket.MSG_PEEK)) == 128
                    received = bytearray()
                    while len(received) < amount:
                        received.extend(peer.recv(min(8192, amount - len(received))))
                    assert received == b"x" * amount
                    peer.sendall(ack)
                    got = b""
                    while len(got) < len(ack):
                        got += client.recv(len(ack) - len(got))
                    assert got == ack
            else:
                assert client.send(b"x" * amount) == amount
                if case == "udp-peek-trunc":
                    buffer = bytearray(64)
                    count, _, _, _ = server.recvmsg_into([buffer], 0, socket.MSG_PEEK | socket.MSG_TRUNC)
                    assert count == amount and buffer == b"x" * 64
                    count, _, flags, _ = server.recvmsg_into([buffer], 0, socket.MSG_TRUNC)
                    assert count == amount and flags & socket.MSG_TRUNC and buffer == b"x" * 64
                    return {"sent": amount, "received": 64}
                data, sender = server.recvfrom(amount + 1)
                assert data == b"x" * amount
                assert server.sendto(ack, sender) == len(ack)
                assert client.recv(64) == ack
    return {"sent": amount + len(ack), "received": amount + len(ack)}


def worker(uid, gid):
    if os.geteuid() == 0 and (uid != 0 or gid != 0):
        os.setgroups([])
        os.setgid(gid)
        os.setuid(uid)
    print(json.dumps({"pid": os.getpid(), "uid": os.getuid()}), flush=True)
    for command in sys.stdin:
        case = command.strip()
        if case == "stop":
            return
        if case == "exit-after-tcp":
            print(json.dumps(workload("tcp4")), flush=True)
            return
        print(json.dumps(workload(case)), flush=True)


class ReadTimeout(RuntimeError):
    pass


class Lines:
    """Read a bounded protocol line with a real monotonic deadline."""
    def __init__(self, stream, label):
        self.fd = stream.fileno()
        self.label = label
        self.pending = bytearray()
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.fd, selectors.EVENT_READ)

    def read(self, timeout=8):
        deadline = time.monotonic() + timeout
        while True:
            offset = self.pending.find(b"\n")
            if offset >= 0:
                line = bytes(self.pending[:offset])
                del self.pending[:offset + 1]
                return json.loads(line)
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise ReadTimeout(f"Timed out waiting for {self.label} output")
            data = os.read(self.fd, 65536)
            if not data:
                raise RuntimeError(f"{self.label} closed its output; inspect {self.label}.log")
            self.pending.extend(data)
            if len(self.pending) > MAX_LINE:
                raise RuntimeError("Collector exceeded its 2 MiB snapshot boundary")

    def close(self):
        self.selector.close()


def observe(reader, target, after_ms, exited, result, timeout=8):
    """Retain bounded diagnostic state even when no byte-total assertion passes."""
    deadline = time.monotonic() + timeout
    result["expected_totals"] = target.copy()
    result["after_ms"] = after_ms
    result["case_snapshots"] = 0
    while time.monotonic() < deadline:
        try:
            sample = reader.read(timeout=max(0.01, deadline - time.monotonic()))
        except ReadTimeout:
            break
        result["case_snapshots"] += 1
        result["snapshots_received"] = result.get("snapshots_received", 0) + 1
        previous_sample = result.get("last_snapshot", {}).get("sample_ms", -1)
        result["last_snapshot"] = sample
        assert sample["started_ms"] == result["started_ms"], "Collector session changed"
        assert sample["sample_ms"] >= previous_sample, "Collector timestamp moved backwards"
        assert not sample["truncated"], "Collector truncated the process snapshot"
        assert sample["lost_operations"] == 0, "Collector reported lost operations"
        assert all(row["pid"] == result["workload"]["pid"] and row["uid"] == result["workload"]["uid"]
                   for row in sample["rows"]), "Collector included a different process/UID"
        actual = {field: sum(row[field] for row in sample["rows"]) for field in target}
        result["observed_totals"] = actual
        if sample["sample_ms"] <= after_ms:
            continue
        assert all(actual[field] <= target[field] for field in target), f"Counters exceeded expected bytes: {actual} > {target}"
        if actual == target:
            if sample["rows"]:
                assert all(row["start_ns"] > 0 and row["inode"] > 0 for row in sample["rows"]), "Missing process lifetime/executable identity"
            return sample
    raise RuntimeError(f"Byte totals did not reach {target}; observed={result.get('observed_totals')}, "
                       f"snapshots={result['case_snapshots']}, "
                       f"last_sample_ms={result.get('last_snapshot', {}).get('sample_ms')}, after_ms={after_ms}")


def check_attribution(sample, exited):
    for row in sample["rows"]:
        assert row["executable_hex"] is not None, (
            f"Unexpected executable path availability for PID {row['pid']}: "
            f"exited={exited}, reason={row.get('executable_unavailable_reason')}")
        if exited:
            assert row.get("executable_source") == "kernel", "Post-exit path was not retained by the kernel metadata capture"


def terminate(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=3)


def short_lived(args, evidence, result, environment, processes, readers):
    """Three real curl execs exit before the first nonempty collector sample."""
    if not Path("/usr/bin/curl").is_file():
        raise RuntimeError("The short-lived regression requires /usr/bin/curl")
    expected_path = os.fsencode(os.path.realpath("/usr/bin/curl")).hex()
    transfers, errors = {}, []
    response_body = b"x" * 65536
    response = (f"HTTP/1.0 200 OK\r\nContent-Length: {len(response_body)}\r\nConnection: close\r\n\r\n".encode()
                + response_body)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.bind(("127.0.0.1", 0))
        server.listen(3)
        server.settimeout(5)
        def serve():
            try:
                for _ in range(3):
                    peer, _ = server.accept()
                    with peer:
                        peer.settimeout(3)
                        request = bytearray()
                        while b"\r\n\r\n" not in request:
                            chunk = peer.recv(4096)
                            if not chunk:
                                raise RuntimeError("Owned curl closed its HTTP request")
                            request.extend(chunk)
                            if len(request) > 16384:
                                raise RuntimeError("Owned HTTP request exceeded 16 KiB")
                        index = int(request.split(b" ", 2)[1].strip(b"/"))
                        assert index in range(3) and index not in transfers
                        peer.sendall(response)
                        transfers[index] = {"sent": len(request), "received": len(response)}
            except Exception as error:
                errors.append(str(error))
        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        with (evidence / "worker.log").open("wb") as worker_log, (evidence / "collector.log").open("wb") as collector_log:
            clients = []
            for index in range(3):
                url = f"http://127.0.0.1:{server.getsockname()[1]}/{index}"
                child = subprocess.Popen([sys.executable, "-I", str(SCRIPT), "--curl-child", url,
                                          "--uid", str(args.uid), "--gid", str(args.gid)],
                                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=worker_log, env=environment)
                clients.append(child)
                processes.append(child)
                lines = Lines(child.stdout, "curl-worker")
                readers.append(lines)
                assert lines.read() == {"pid": child.pid, "uid": args.uid}
            collector_lines = None
            if not args.workloads_only:
                build = args.build.resolve()
                manifest = json.loads((build / "build.json").read_text())
                assert manifest["status"] == "passed" and (manifest.get("portable") or manifest["kernel"] == platform.release())
                for name, digest in manifest["sha256"].items():
                    assert hashlib.sha256((build / name).read_bytes()).hexdigest() == digest, f"Build changed: {name}"
                command = [str(build / "collector"), "--object", str(build / "accounting.bpf.o"),
                           "--uid", str(args.uid), "--seconds", "15", "--interval-ms", "5000"]
                for client in clients:
                    command += ["--pid", str(client.pid)]
                result["command"] = command
                collector = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=collector_log, env=environment)
                processes.append(collector)
                collector_lines = Lines(collector.stdout, "collector")
                readers.append(collector_lines)
                first = collector_lines.read()
                assert first["rows"] == [] and first["lost_operations"] == 0 and not first["truncated"]
                result.update(loaded_bpf=True, started_ms=first["started_ms"], last_snapshot=first, snapshots_received=1)
            result["stage"] = "running-short-lived-curl"
            for client in clients:
                client.stdin.write(b"go\n")
                client.stdin.flush()
                client.stdin.close()
                assert client.wait(timeout=4) == 0, "Owned curl failed"
            exited_ms = time.monotonic_ns() // 1_000_000
            thread.join(timeout=3)
            assert not thread.is_alive() and not errors and len(transfers) == 3, f"Owned HTTP server failed: {errors}"
            expected = {field: sum(row[field] for row in transfers.values()) for field in ("sent", "received")}
            result.update(expected_totals=expected, all_clients_exited_ms=exited_ms)
            if collector_lines:
                result["stage"] = "checking-short-lived-attribution"
                for _ in range(2):
                    sample = collector_lines.read(timeout=8)
                    result["last_snapshot"] = sample
                    result["snapshots_received"] += 1
                    assert sample["sample_ms"] > exited_ms, "A curl survived until the first usage sample"
                    assert not sample["truncated"] and sample["lost_operations"] == 0
                    assert len(sample["rows"]) == 3 and {row["pid"] for row in sample["rows"]} == {c.pid for c in clients}
                    assert sample["started_ms"] == result["started_ms"]
                    check_attribution(sample, True)
                    actual = {field: sum(row[field] for row in sample["rows"]) for field in expected}
                    assert actual == expected, f"Exact curl bytes: {actual} != {expected}"
                    for index, client in enumerate(clients):
                        row = next(row for row in sample["rows"] if row["pid"] == client.pid)
                        assert row["uid"] == args.uid and row["executable_hex"] == expected_path and row["exec_id"] > 0
                        assert all(row[field] == transfers[index][field] for field in expected)
                    result["observed_totals"] = actual
                result["all_clients_exited_before_first_usage_sample"] = True
            for index, client in enumerate(clients):
                result["cases"].append({"case": f"short-lived-curl-{index}", "pid": client.pid,
                                        "expected": transfers[index], "status": "passed",
                                        "bytes_status": "passed" if collector_lines else "not-measured"})
            result["byte_cases_passed"] = 3 if collector_lines else 0


def inside(args):
    evidence = args.evidence
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    result = {"status": "failed", "kernel": platform.release(), "cases": [],
              "verifier_sha256": hashlib.sha256(SCRIPT.read_bytes()).hexdigest(),
              "workloads_only": args.workloads_only, "loaded_bpf": False,
              "parent_network_namespace": args.parent_namespace,
              "network_namespace": os.readlink("/proc/self/ns/net"),
              "changed_daemon": False, "owned_cleanup": False}
    processes, readers = [], []
    worker_log = None
    try:
        if result["network_namespace"] == args.parent_namespace:
            raise RuntimeError("Refusing workloads in the parent's network namespace")
        network = subprocess.run(["ip", "link", "set", "lo", "up"], capture_output=True, text=True)
        if network.returncode:
            raise RuntimeError(f"Cannot enable owned loopback: {network.stderr.strip()}")
        environment = {key: value for key, value in os.environ.items() if not key.startswith(("LD_", "PYTHON"))}
        if args.short_lived:
            short_lived(args, evidence, result, environment, processes, readers)
            result.update(status="passed", stage="complete")
            return 0
        result["stage"] = "starting-worker"
        worker_log = (evidence / "worker.log").open("wb")
        child = subprocess.Popen([sys.executable, "-I", str(SCRIPT), "--worker", "--uid", str(args.uid), "--gid", str(args.gid)],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=worker_log, env=environment)
        processes.append(child)
        worker_lines = Lines(child.stdout, "worker")
        readers.append(worker_lines)
        identity = worker_lines.read()
        if identity != {"pid": child.pid, "uid": args.uid}:
            raise RuntimeError(f"Unexpected workload identity: {identity}")
        result["workload"] = identity
        collector_lines = None
        log = (evidence / "collector.log").open("wb")
        try:
            if not args.workloads_only:
                result["stage"] = "starting-collector"
                build = args.build.resolve()
                manifest = json.loads((build / "build.json").read_text())
                if manifest["status"] != "passed" or (not manifest.get("portable") and manifest["kernel"] != platform.release()):
                    raise RuntimeError("Collector build does not match the running kernel")
                for name, digest in manifest["sha256"].items():
                    if hashlib.sha256((build / name).read_bytes()).hexdigest() != digest:
                        raise RuntimeError(f"Collector build changed: {name}")
                command = [str(build / "collector"), "--object", str(build / "accounting.bpf.o"),
                           "--uid", str(args.uid), "--pid", str(child.pid), "--seconds", "45", "--interval-ms", "100"]
                result["command"] = command
                collector = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=log, env=environment)
                processes.append(collector)
                collector_lines = Lines(collector.stdout, "collector")
                readers.append(collector_lines)
                first = collector_lines.read()
                assert first["version"] == 1 and first["rows"] == []
                assert first["lost_operations"] == 0 and not first["truncated"]
                result["loaded_bpf"] = True
                result["started_ms"] = first["started_ms"]
                result["last_snapshot"] = first
                result["snapshots_received"] = 1
            expected = {"sent": 0, "received": 0}
            attribution_errors = []
            for case in (*CASES, "exit-after-tcp"):
                result["current_case"] = case
                result["stage"] = "waiting-for-workload"
                child.stdin.write((case + "\n").encode())
                child.stdin.flush()
                delta = worker_lines.read()
                result["last_workload_delta"] = delta
                for field in expected:
                    expected[field] += delta[field]
                exited = case == "exit-after-tcp"
                if exited:
                    assert child.wait(timeout=3) == 0
                completed_ms = time.monotonic_ns() // 1_000_000
                result["stage"] = "waiting-for-counters"
                sample = observe(collector_lines, expected, completed_ms, exited, result) if collector_lines else None
                record = {"case": case, "delta": delta, "expected": expected.copy(), "snapshot": sample, "status": "passed"}
                if sample:
                    record["bytes_status"] = "passed"
                    try:
                        check_attribution(sample, exited)
                        record["attribution_status"] = "passed"
                    except AssertionError as error:
                        # Keep the check strict, but finish independent byte cases
                        # before reporting a known display-path failure.
                        record.update(status="failed", attribution_status="failed", error=str(error))
                        attribution_errors.append(f"{case}: {error}")
                result["cases"].append(record)
            if collector_lines:
                result["stage"] = "checking-idle-counters"
                # Idle snapshots must retain totals without inventing extra traffic.
                observe(collector_lines, expected, time.monotonic_ns() // 1_000_000, True, result)
                observe(collector_lines, expected, time.monotonic_ns() // 1_000_000, True, result)
            result["byte_cases_passed"] = sum(case.get("bytes_status") == "passed" for case in result["cases"])
            if attribution_errors:
                result["stage"] = "checking-executable-attribution"
                raise RuntimeError("Executable attribution failed after byte checks: " + "; ".join(attribution_errors))
        finally:
            for process in reversed(processes):
                terminate(process)
            log.close()
        result["status"] = "passed"
        result["stage"] = "complete"
    except (Exception, KeyboardInterrupt) as error:
        result["error"] = str(error) or type(error).__name__
        (evidence / "failure.log").write_text(traceback.format_exc())
    finally:
        for process in reversed(processes):
            terminate(process)
        for reader in readers:
            reader.close()
        if worker_log:
            worker_log.close()
        result["process_exit_codes"] = [p.returncode for p in processes]
        result["owned_cleanup"] = all(p.poll() is not None for p in processes)
        for name in ("collector.log", "worker.log", "failure.log"):
            path = evidence / name
            if path.exists():
                path.chmod(0o600)
        report = evidence / "result.json"
        report.write_text(json.dumps(result, indent=2) + "\n")
        report.chmod(0o600)
        # Make root-created evidence readable by the invoking user only.
        if os.geteuid() == 0 and args.uid != 0:
            for path in (evidence, *evidence.iterdir()):
                os.chown(path, args.uid, args.gid)
        print(json.dumps({"status": result["status"], "loaded_bpf": result["loaded_bpf"],
                          "cases": len(result["cases"]), "byte_cases_passed": result.get("byte_cases_passed", 0),
                          "report": str(report), "error": result.get("error")}))
    return 0 if result["status"] == "passed" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", type=Path)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--workloads-only", action="store_true")
    parser.add_argument("--short-lived", action="store_true", help="Three owned curl processes exiting before the first usage sample")
    parser.add_argument("--curl-child", help=argparse.SUPPRESS)
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--parent-namespace", help=argparse.SUPPRESS)
    parser.add_argument("--uid", type=int, default=int(os.environ.get("SUDO_UID", os.getuid())))
    parser.add_argument("--gid", type=int, default=int(os.environ.get("SUDO_GID", os.getgid())))
    args = parser.parse_args()
    if args.curl_child:
        if os.geteuid() == 0 and (args.uid != 0 or args.gid != 0):
            os.setgroups([])
            os.setgid(args.gid)
            os.setuid(args.uid)
        print(json.dumps({"pid": os.getpid(), "uid": os.getuid()}), flush=True)
        assert sys.stdin.readline().strip() == "go"
        os.execv("/usr/bin/curl", ["/usr/bin/curl", "--silent", "--show-error", "--noproxy", "*",
                                 "--http1.0", "--output", "/dev/null", "--url", args.curl_child])
    if args.worker:
        worker(args.uid, args.gid)
        return 0
    if args.inside:
        if not args.parent_namespace or not args.evidence:
            parser.error("Internal namespace/evidence arguments are required")
        return inside(args)
    if not args.workloads_only and (not args.build or os.geteuid() != 0):
        parser.error("Live BPF verification needs root and --build DIRECTORY; --workloads-only needs neither")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    evidence = (args.evidence or ROOT / ".verification/bandwidth" / f"run-{stamp}").resolve()
    command = ["unshare", "--net"]
    if os.geteuid() != 0:
        command += ["--user", "--map-root-user"]
    command += [sys.executable, "-I", str(SCRIPT), "--inside", "--parent-namespace", os.readlink("/proc/self/ns/net"),
                "--evidence", str(evidence), "--uid", str(0 if os.geteuid() != 0 else args.uid),
                "--gid", str(0 if os.geteuid() != 0 else args.gid)]
    if args.short_lived:
        command.append("--short-lived")
    if args.workloads_only:
        command.append("--workloads-only")
    else:
        command += ["--build", str(args.build.resolve())]
    return subprocess.call(command)


if __name__ == "__main__":
    raise SystemExit(main())
