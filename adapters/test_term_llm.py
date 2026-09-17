"""Offline adapter tests; optional pinned-binary test uses only a localhost stub."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch
from types import SimpleNamespace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from term_llm_adapter import guest_config, normalize
import term_llm_adapter as adapter


def stream(*events):
    return b"".join(json.dumps(e).encode() + b"\n" for e in events)


START = {"type": "session.started"}
STATS = {"type": "stats", "input_tokens": 12, "output_tokens": 4, "llm_calls": 1}
DONE = {"type": "done", "tokens": 4}


class NormalizeTests(unittest.TestCase):
    def test_input_includes_cache_without_double_counting_final_stats(self):
        usage = {"type": "usage", "input_tokens": 3, "output_tokens": 4, "cached_input_tokens": 100, "cache_write_tokens": 20}
        self.assertEqual(normalize(stream(usage))[2]["input_tokens"], 123)
        stats = dict(usage, type="stats")
        self.assertEqual(normalize(stream(usage, stats, DONE))[2]["input_tokens"], 123)

    def test_cumulative_stats_are_not_double_counted(self):
        delta = dict(STATS, type="usage")
        status, _, usage, transcript = normalize(stream(START, delta, STATS, DONE), 0)
        self.assertEqual(status, "completed")
        self.assertEqual(usage["input_tokens"], 12)
        self.assertEqual(usage["output_tokens"], 4)
        self.assertIsNone(usage["cost_microusd"])
        self.assertIsNone(transcript["usage"]["cost_usd"])

    def test_errors_override_done_and_zero_exit(self):
        for message, expected in [("HTTP 429 quota", "unavailable"), ("HTTP 503", "unavailable"),
                                  ("canceled", "interrupted"), ("max turns exceeded", "failed"),
                                  ("max turns exceeded: 500", "failed"),
                                  ("unexpected error", "harness_error")]:
            data = stream(START, {"type": "error", "message": message}, STATS, DONE)
            self.assertEqual(normalize(data, 0)[0], expected)

    def test_reboot_and_unknown_disconnect(self):
        data = stream(START, STATS, DONE)
        self.assertEqual(normalize(data, 255, True)[0], "interrupted")
        for code in (255, 137):
            self.assertEqual(normalize(data, code)[0], "harness_error")
        for code in (130, 143, -15):
            self.assertEqual(normalize(data, code)[0], "interrupted")

    def test_partial_usage_and_truncated_tail(self):
        data = stream({"type": "usage", "input_tokens": 2, "output_tokens": 3}) + b'{"type":'
        self.assertEqual(normalize(data)[2]["input_tokens"], 2)
        self.assertEqual(normalize(data, 0)[0], "harness_error")
        self.assertEqual(normalize(stream(DONE), 0)[0], "harness_error")
        self.assertIsNone(normalize(b"")[2]["input_tokens"])
        for data in (b"broken\n", b"[]\n", b"{}\n"):
            with self.assertRaises(ValueError):
                normalize(data)

    def test_tool_trace(self):
        data = stream(dict(START, ts="2026-09-15T00:00:00Z"),
                      {"type": "tool.started", "call_id": "a", "name": "shell", "args": {"command": "pwd"}, "ts": "2026-09-15T00:00:01Z"},
                      {"type": "tool.completed", "call_id": "a", "success": False, "info": "failed", "ts": "2026-09-15T00:00:03Z"})
        tool = normalize(data)[3]["tool_trace"][0]
        self.assertEqual(tool["duration_ms"], 2000)
        self.assertEqual(tool["started_after_ms"], 1000)
        self.assertTrue(tool["is_error"])
        self.assertFalse(tool["read_only"])

    def test_exact_model_and_effort(self):
        cfg = guest_config("vendor/model-high", "high", 1234)["providers"]["arena"]
        self.assertEqual(cfg["models"][0]["id"], "vendor/model-high")
        self.assertEqual(cfg["models"][0]["default_reasoning_effort"], "high")
        self.assertEqual(cfg["api_key"], "arena-proxy-placeholder")
        self.assertEqual(cfg["base_url"], "http://127.0.0.1:1234")
        cfg = guest_config("vendor/model", "default", 1234)
        self.assertNotIn("default_reasoning_effort", cfg["providers"]["arena"]["models"][0])
        with self.assertRaises(ValueError):
            guest_config("vendor/model", "invalid", 1234)


class LauncherTests(unittest.TestCase):
    def test_proxy_boundary_and_terminal_result(self):
        for provider, key_name, endpoint in [("openrouter", "OPENROUTER_API_KEY", "https://openrouter.ai/api/v1"),
                                             ("vercel", "AI_GATEWAY_API_KEY", "https://ai-gateway.vercel.sh/v1")]:
            with self.subTest(provider=provider), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                proxy = root / "proxy.py"
                proxy.touch()
                env = {"AOE_AGENT_ID": "test", "AOE_TERRITORY_ID": "builder-one", "AOE_TERRITORY_HOST": "127.0.0.1",
                       "AOE_SSH_PORT": "26000", "AOE_MODEL": provider + "/vendor/model", "AOE_REASONING_EFFORT": "high",
                       "AOE_INSTRUCTION_FILE": str(root / "instruction.md"), "AOE_RESULT_FILE": str(root / "result.json"),
                       "AOE_USAGE_FILE": str(root / "usage.json"), "AOE_CREDENTIAL_FILE": str(root / "credentials.env"),
                       "AOE_TERM_LLM_PROXY": str(proxy), "AOE_PLAYER_ARTIFACTS_JSON": '[]'}
                launched, copied = [], []

                class Child:
                    returncode = None

                    def __init__(self, args, **kwargs):
                        launched.append((args, kwargs))
                        if "--ready-file" in args:
                            Path(args[args.index("--ready-file") + 1]).write_text("12345")
                        if args[0] == "ssh" and "ask" in args[-1]:
                            self.returncode = 0

                    def poll(self):
                        return self.returncode

                    def terminate(self):
                        self.returncode = -15

                    def wait(self, **kwargs):
                        return self.returncode

                def run(args, **kwargs):
                    if args[0] == "bash":
                        return SimpleNamespace(returncode=0, stdout=f"{key_name}=test-secret\0AOE_SSH_PASSWORD=test-password\0PATH=/usr/bin\0".encode())
                    if args[0] == "scp":
                        copied.append(args)
                        if args[-1].endswith("events.partial"):
                            Path(args[-1]).write_bytes(stream(START, STATS, DONE))
                    return SimpleNamespace(returncode=0)

                with patch.dict(os.environ, env, clear=True), patch.object(adapter, "binary", return_value=root / "binary"), \
                     patch.object(adapter.subprocess, "run", side_effect=run), patch.object(adapter.subprocess, "Popen", Child), \
                     patch.object(adapter.signal, "signal"), patch.object(adapter.time, "sleep"):
                    self.assertEqual(adapter.main(), 0)
                result = json.loads((root / "result.json").read_text())
                self.assertEqual(result["status"], "completed")
                self.assertEqual(result["usage"]["input_tokens"], 12)
                self.assertIsNone(result["usage"]["cost_microusd"])
                config = json.loads((root / "term-llm-config.yaml").read_text())
                self.assertEqual(config["providers"]["arena"]["api_key"], "arena-proxy-placeholder")
                self.assertEqual(launched[0][1]["env"]["REPLAYBOOK_OPENAI_API_KEY"], "test-secret")
                self.assertIn(endpoint, launched[0][0])
                self.assertNotIn("test-secret", json.dumps([args for args, _ in launched]))
                self.assertNotIn(str(root / "credentials.env"), json.dumps(copied))
                command = launched[-1][0][-1]
                self.assertIn("--approval yolo", command)
                self.assertIn("--skills none", command)
                self.assertIn("--no-session", command)

    def test_checksum_binding(self):
        import hashlib
        helper = Path(adapter.__file__)
        self.assertIn(hashlib.sha256(helper.read_bytes()).hexdigest(), helper.with_name("term-llm.sh").read_text())


@unittest.skipUnless(os.environ.get("AOE_TEST_TERM_LLM_BINARY"), "optional pinned binary localhost integration")
class BinaryTests(unittest.TestCase):
    def test_real_cli_routes_exact_model_and_effort(self):
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                requests.append((self.path, self.headers.get("Authorization"), json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                chunks = [{"id": "stub", "choices": [{"index": 0, "delta": {"content": "Done."}, "finish_reason": "stop"}]},
                          {"id": "stub", "choices": [], "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16}}]
                if not any(m.get("role") == "tool" for m in requests[-1][2]["messages"]):
                    chunks[0]["choices"][0] = {"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call-stub", "type": "function", "function": {"name": "shell", "arguments": json.dumps({"command": "pwd", "working_dir": directory})}}]}, "finish_reason": "tool_calls"}
                for chunk in chunks:
                    self.wfile.write(b"data: " + json.dumps(chunk).encode() + b"\n\n")
                self.wfile.write(b"data: [DONE]\n\n")

        with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server, tempfile.TemporaryDirectory() as directory:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                root = Path(directory)
                (root / "term-llm").mkdir()
                for effort in ("default", "high"):
                    (root / "term-llm/config.yaml").write_text(json.dumps(guest_config("vendor/exact-model", effort, server.server_port)))
                    env = {"PATH": os.environ.get("PATH", ""), "XDG_CONFIG_HOME": directory,
                           "XDG_DATA_HOME": directory, "XDG_CACHE_HOME": directory}
                    result = subprocess.run([os.environ["AOE_TEST_TERM_LLM_BINARY"], "ask", "--json", "--no-session", "--provider", "arena:entrant", "--approval", "yolo", "--tools", "read_file,write_file,edit_file,shell,grep,glob", "--skills", "none", "--no-search", "--max-turns", "2", "--", "Say done."],
                                            cwd=directory, env=env, capture_output=True, timeout=30)
                    self.assertEqual(result.returncode, 0, result.stderr.decode())
                    self.assertEqual(normalize(result.stdout, 0)[0], "completed", result.stdout)
                    self.assertEqual(normalize(result.stdout, 0)[2]["input_tokens"], 24)
                    self.assertEqual(len(normalize(result.stdout, 0)[3]["tool_trace"]), 1)
                    self.assertFalse(normalize(result.stdout, 0)[3]["tool_trace"][0]["is_error"])
                    path, auth, body = requests[-1]
                    self.assertEqual(path, "/chat/completions")
                    self.assertEqual(auth, "Bearer arena-proxy-placeholder")
                    self.assertEqual(body["model"], "vendor/exact-model")
                    if effort == "default":
                        self.assertNotIn("reasoning_effort", body)
                    else:
                        self.assertEqual(body["reasoning_effort"], effort)
                    self.assertTrue(any(t["function"]["name"] == "shell" for t in body["tools"]))
            finally:
                server.shutdown()
                thread.join()
