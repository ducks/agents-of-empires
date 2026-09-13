"""Preflight tests never query providers or start VMs."""
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

from opencode_adapter import go_model, preflight


class PreflightTests(unittest.TestCase):
    def test_launcher_forwards_preflight_arguments(self):
        result = subprocess.run(["bash", str(Path(__file__).with_name("opencode.sh")), "--preflight", "opencode-go/not-a-model"], capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 2)
        self.assertIn("no pinned definition", result.stderr)
        self.assertNotIn("AOE_AGENT_ID", result.stderr)

    def test_missing_definition_fails_without_network_or_binary(self):
        with patch("opencode_adapter.subprocess.run") as run:
            with self.assertRaisesRegex(ValueError, "no pinned definition"):
                preflight(["opencode-go/not-a-model"])
            run.assert_not_called()

    def test_missing_provider_model_fails_before_client(self):
        reply = subprocess.CompletedProcess([], 0, json.dumps({"data": []}), "")
        with patch("opencode_adapter.subprocess.run", return_value=reply), patch("opencode_adapter.binary") as binary:
            with self.assertRaisesRegex(ValueError, "does not advertise"):
                preflight(["opencode-go/deepseek-v4.1-flash"])
            binary.assert_not_called()

    def test_client_must_resolve_every_model_without_credentials(self):
        catalog = subprocess.CompletedProcess([], 0, json.dumps({"data": [{"id": "deepseek-v4.1-flash"}]}), "")
        for output, succeeds in [("", False), ("opencode-go/deepseek-v4.1-flash\n", True)]:
            client = subprocess.CompletedProcess([], 0, output, "")
            with patch("opencode_adapter.subprocess.run", side_effect=[catalog, client]) as run, patch("opencode_adapter.binary", return_value="opencode"):
                if succeeds:
                    preflight(["opencode-go/deepseek-v4.1-flash"])
                else:
                    with self.assertRaisesRegex(ValueError, "cannot resolve"):
                        preflight(["opencode-go/deepseek-v4.1-flash"])
                call = run.call_args
                self.assertEqual(call.args[0], ["opencode", "models", "opencode-go"])
                self.assertNotIn("OPENCODE_API_KEY", call.kwargs["env"])
                self.assertEqual(call.kwargs["env"]["OPENCODE_DISABLE_MODELS_FETCH"], "true")
        self.assertEqual(go_model("opencode-go/deepseek-v4.1-flash")[1]["interleaved"], {"field": "reasoning_content"})
