"""Exercise the real adapter against fake SSH, SCP and credential proxy processes."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class TransportTests(unittest.TestCase):
    def run_case(self, code, error=False, reboot=False):
        with tempfile.TemporaryDirectory(prefix="aoe-opencode-test-") as directory:
            root = Path(directory)
            ssh = root / "ssh"
            ssh.write_text('''#!/usr/bin/env python3
import os, sys, time
from pathlib import Path
if "-N" in sys.argv:
    time.sleep(60)
elif "events.jsonl" in sys.argv[-1]:
    Path(os.environ["TEST_COMMAND"]).write_text(sys.argv[-1])
    if os.environ["TEST_REBOOT"] == "1":
        Path(os.environ["AOE_RESULT_FILE"]).with_name("referee-reboot").touch()
    sys.exit(int(os.environ["TEST_EXIT"]))
''')
            scp = root / "scp"
            scp.write_text('''#!/usr/bin/env python3
import os, sys
from pathlib import Path
if sys.argv[-2].endswith("/events.jsonl"):
    Path(sys.argv[-1]).write_text(os.environ["TEST_EVENTS"])
''')
            proxy = root / "proxy.py"
            proxy.write_text('''import sys, time
from pathlib import Path
Path(sys.argv[sys.argv.index("--ready-file")+1]).write_text("41000")
time.sleep(60)
''')
            ssh.chmod(0o700)
            scp.chmod(0o700)
            credential = root / "credential.env"
            credential.write_text("OPENCODE_API_KEY=test-controller-secret\nAOE_SSH_PASSWORD=test-password\n")
            credential.chmod(0o600)
            instruction = root / "instruction.md"
            instruction.write_text("Fix the service")
            events = [{"type": "step_finish", "part": {"reason": "stop", "tokens": {"input": 12, "output": 4}, "cost": .25}}]
            if error:
                events.append({"type": "error", "error": {"name": "APIError", "data": {"statusCode": 429, "message": "busy"}}})
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                       AOE_AGENT_ID="test", AOE_TERRITORY_ID="one", AOE_TERRITORY_HOST="127.0.0.1", AOE_SSH_PORT="26000",
                       AOE_MODEL="opencode-go/test-model", AOE_REASONING_EFFORT="high", AOE_INSTRUCTION_FILE=str(instruction),
                       AOE_CREDENTIAL_FILE=str(credential), AOE_RESULT_FILE=str(root / "result.json"),
                       AOE_USAGE_FILE=str(root / "usage.json"), AOE_OPENCODE_BINARY="/bin/true", AOE_OPENCODE_PROXY=str(proxy),
                       TEST_EXIT=str(code), TEST_EVENTS="".join(json.dumps(e)+"\n" for e in events),
                       TEST_REBOOT="1" if reboot else "0", TEST_COMMAND=str(root / "command.txt"))
            process = subprocess.run(["bash", str(Path(__file__).with_name("opencode.sh"))], env=env,
                                     capture_output=True, timeout=20)
            result = json.loads((root / "result.json").read_text())
            for artifact in ("result.json", "usage.json", "transcript.json", "opencode-config.json", "command.txt"):
                self.assertNotIn("test-controller-secret", (root / artifact).read_text())
            self.assertNotIn(b"test-controller-secret", process.stdout + process.stderr)
            config = json.loads((root / "opencode-config.json").read_text())
            self.assertEqual(config["enabled_providers"], ["opencode-go"])
            self.assertTrue(config["provider"]["opencode-go"]["options"]["baseURL"].startswith("http://127.0.0.1:"))
            command = (root / "command.txt").read_text()
            self.assertIn("--variant high", command)
            self.assertIn("--format json", command)
            return result

    def test_success(self):
        result = self.run_case(0)
        self.assertEqual(result["status"], "completed")
        self.assertEqual(result["usage"]["input_tokens"], 12)

    def test_error_overrides_zero_exit(self):
        self.assertEqual(self.run_case(0, error=True)["status"], "unavailable")

    def test_reboot_keeps_usage(self):
        result = self.run_case(255, reboot=True)
        self.assertEqual(result["status"], "interrupted")
        self.assertEqual(result["usage"]["output_tokens"], 4)
