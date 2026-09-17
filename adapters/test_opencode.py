"""Offline normalization tests; no provider keys, VM, or inference required."""
import json
import unittest

from opencode_adapter import normalize


def stream(*events):
    return b"".join(json.dumps(event).encode() + b"\n" for event in events)


STEP = {"type": "step_finish", "part": {"reason": "stop", "tokens": {"input": 12, "output": 4}, "cost": .25}}


class NormalizeTests(unittest.TestCase):
    def test_input_includes_cache_reads_and_writes(self):
        step = {"type": "step_finish", "part": {"reason": "stop", "tokens": {"input": 3, "output": 4, "cache": {"read": 100, "write": 20}}}}
        self.assertEqual(normalize(stream(step), 0)[2]["input_tokens"], 123)

    def test_unknown_server_error_and_sigkill_are_not_player_failures(self):
        error = {"type": "error", "error": {"name": "UnknownError", "data": {"message": "Unexpected server error. Check server logs for details."}}}
        self.assertEqual(normalize(stream(error), 0)[0], "harness_error")
        for data in (b"", stream(STEP)):
            self.assertEqual(normalize(data, 137)[0], "harness_error")
        self.assertEqual(normalize(stream(STEP), 137)[2]["input_tokens"], 12)

    def test_success_keeps_estimate_separate_from_actual_spend(self):
        status, text, usage, transcript = normalize(stream(STEP, {"type": "text", "part": {"text": "done"}}), 0)
        self.assertEqual((status, text), ("completed", "done"))
        self.assertEqual(usage["input_tokens"], 12)
        self.assertIsNone(usage["cost_microusd"])
        self.assertEqual(transcript["usage"]["estimated_cost_usd"], .25)

    def test_error_beats_zero_exit_and_preserves_usage(self):
        for code in (401, 402, 403, 404, 408, 429, 500, 503):
            error = {"type": "error", "error": {"name": "APIError", "data": {"statusCode": code, "message": "provider error"}}}
            status, _, usage, _ = normalize(stream(STEP, error), 0)
            self.assertEqual(status, "unavailable")
            self.assertEqual(usage["output_tokens"], 4)

    def test_context_error_is_player_failure(self):
        error = {"type": "error", "error": {"name": "ContextOverflowError", "data": {"message": "too long"}}}
        self.assertEqual(normalize(stream(error), 0)[0], "failed")

    def test_reboot_and_unexplained_disconnect_are_different(self):
        data = stream(STEP) + b'{"type":'
        self.assertEqual(normalize(data, 255, True)[0], "interrupted")
        self.assertEqual(normalize(data, 255)[0], "harness_error")
        self.assertEqual(normalize(data, 255)[2]["input_tokens"], 12)

    def test_missing_or_tool_only_output_is_not_success(self):
        self.assertEqual(normalize(b"", 0)[0], "harness_error")
        step = {"type": "step_finish", "part": {"reason": "tool-calls"}}
        self.assertEqual(normalize(stream(step), 0)[0], "failed")
        self.assertIsNone(normalize(stream(step), 0)[2]["input_tokens"])

    def test_malformed_complete_line_is_rejected(self):
        with self.assertRaises(ValueError):
            normalize(b'{broken}\n')

    def test_tool_error_is_not_terminal_error_and_reasoning_is_excluded(self):
        data = stream({"type": "reasoning", "part": {"text": "private"}},
                      {"type": "tool_use", "timestamp": 100, "part": {"tool": "bash", "state": {
                          "status": "error", "input": {"command": "false"}, "error": "exit 1",
                          "time": {"start": 100, "end": 110}}}}, STEP)
        status, _, usage, transcript = normalize(data, 0)
        self.assertEqual(status, "completed")
        self.assertEqual(usage["tool_calls"], 1)
        self.assertTrue(transcript["tool_trace"][0]["is_error"])
        self.assertNotIn("private", json.dumps(transcript))


if __name__ == "__main__":
    unittest.main()
