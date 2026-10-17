#!/usr/bin/env python3
"""Exercise the real C Unix-stream connector without loading BPF or using sudo."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest


class TransportTests(unittest.TestCase):
    binary = None

    def exchange(self, uid):
        with tempfile.TemporaryDirectory(prefix="usage-transport-") as directory:
            path = str(Path(directory) / "receiver.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(path)
                server.listen(1)
                server.settimeout(2)
                process = subprocess.run([str(self.binary), "--connect-test", path, str(uid)],
                                         capture_output=True, timeout=3)
                connection, _ = server.accept()
                with connection:
                    connection.settimeout(2)
                    data = connection.recv(4096)
                return process, data

    def test_stream_goes_to_owned_receiver(self):
        process, data = self.exchange(os.getuid())
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(process.stdout, b"")
        self.assertEqual(data, b"transport-fixture\n")

    def test_wrong_receiver_uid_is_rejected(self):
        process, data = self.exchange(os.getuid() + 1)
        self.assertEqual(process.returncode, 1)
        self.assertEqual(data, b"")
        self.assertIn(b"does not belong to the selected UID", process.stderr)

    def test_relative_and_long_socket_paths_are_rejected(self):
        for path in ("relative.sock", "/" + "x" * 108):
            with self.subTest(path=path):
                process = subprocess.run([str(self.binary), "--connect-test", path, str(os.getuid())],
                                         capture_output=True, timeout=3)
                self.assertEqual(process.returncode, 1)
                self.assertIn(b"short absolute Unix socket path", process.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", required=True, type=Path)
    args = parser.parse_args()
    TransportTests.binary = args.build.resolve() / "test-collector"
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(TransportTests))
    report = {"status": "passed" if result.wasSuccessful() else "failed", "tests": result.testsRun,
              "loaded_bpf": False, "uid": os.getuid(), "boundary": "Actual C connect_output, same-user Unix stream, peer UID and path refusal; no BPF load"}
    output = args.build / "transport-tests.json"
    output.write_text(json.dumps(report, indent=2) + "\n")
    output.chmod(0o600)
    print(json.dumps(report))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
