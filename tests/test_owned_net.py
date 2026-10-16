"""The loopback setup must refuse the host namespace before executing ip."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("owned_net", Path(__file__).resolve().parents[1] / "scripts/owned_net.py")
owned_net = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owned_net)


class OwnedNetworkGuards(unittest.TestCase):
    def test_same_namespace_never_changes_interfaces(self):
        with tempfile.TemporaryDirectory() as directory:
            args = ["owned_net.py", "--inside", "--parent", "same", "--evidence", str(Path(directory) / "network.json"), "--", "true"]
            with patch("sys.argv", args), patch.object(owned_net.os, "readlink", return_value="same"), patch.object(Path, "read_text", return_value="0 1000 1"), patch.object(owned_net.subprocess, "run") as run:
                with self.assertRaisesRegex(RuntimeError, "Refusing fixture setup"):
                    owned_net.main()
                run.assert_not_called()

    def test_extra_interface_never_changes_loopback(self):
        args = ["owned_net.py", "--inside", "--parent", "parent", "--evidence", "/tmp/unused-network-proof.json", "--", "true"]
        with patch("sys.argv", args), patch.object(owned_net.os, "readlink", return_value="owned"), patch.object(Path, "read_text", return_value="0 1000 1"), patch.object(owned_net.subprocess, "check_output", return_value=b'[{"ifname":"lo"},{"ifname":"eth0"}]'), patch.object(owned_net.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "other than its owned loopback"):
                owned_net.main()
            run.assert_not_called()

    def test_owned_namespace_records_proof_and_runs_exact_command(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "network.json"
            args = ["owned_net.py", "--inside", "--parent", "parent", "--evidence", str(output), "--", "fixture", "--only-local"]
            with patch("sys.argv", args), patch.object(owned_net.os, "readlink", return_value="owned"), patch.object(Path, "read_text", return_value="0 1000 1"), patch.object(owned_net.subprocess, "check_output", return_value=b'[{"ifname":"lo"}]'), patch.object(owned_net.subprocess, "run") as run, patch.object(owned_net.subprocess, "call", return_value=0) as call:
                self.assertEqual(owned_net.main(), 0)
                run.assert_called_once_with(["ip", "link", "set", "lo", "up"], check=True)
                call.assert_called_once_with(["fixture", "--only-local"])
            proof = json.loads(output.read_text())
            self.assertEqual(proof["network_namespace"], "owned")
            self.assertNotEqual(proof["network_namespace"], proof["parent_network_namespace"])


if __name__ == "__main__":
    unittest.main()
