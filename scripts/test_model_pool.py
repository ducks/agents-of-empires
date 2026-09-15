"""Pure refresh tests: no network, credentials, or inference."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("refresh_pool", Path(__file__).with_name("refresh-model-pool.py"))
pool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pool)


class RefreshTests(unittest.TestCase):
    def inputs(self):
        registry = {"schema_version": 1, "checked_at": "2026-09-14", "models": [{
            "id": "luna", "name": "Luna", "family": "openai", "routes": {
                "vercel": {"model": "openai/luna", "reasoning_effort": "high", "status": "blocked", "listed": True, "note": "quota", "pricing": {}}
            }}]}
        model = {"id": "openai/luna", "type": "language", "tags": ["tool-use"], "supported_parameters": ["tools"], "pricing": {"input": "0.2"}}
        catalogs = {"vercel": [model], "openrouter": [model], "opencode-go": [{"id": "gpt-luna"}]}
        definitions = {"gpt-luna": {"id": "gpt-luna", "tool_call": True}}
        return registry, catalogs, definitions

    def test_keeps_policy_and_adds_new_routes_disabled(self):
        original, catalogs, definitions = self.inputs()
        result, changes = pool.refresh(original, catalogs, definitions)
        luna = next(m for m in result["models"] if m["id"] == "luna")
        self.assertEqual(luna["routes"]["vercel"]["status"], "blocked")
        self.assertEqual(luna["routes"]["vercel"]["reasoning_effort"], "high")
        self.assertEqual(luna["routes"]["openrouter"]["status"], "disabled")
        self.assertNotIn("openrouter", original["models"][0]["routes"])
        again, _ = pool.refresh(result, catalogs, definitions)
        self.assertEqual(again, result)
        self.assertTrue(changes)

    def test_delisted_routes_are_retained_without_substitution(self):
        registry, catalogs, definitions = self.inputs()
        catalogs["vercel"][0] = {"id": "vendor/replacement", "type": "language", "tags": ["tool-use"]}
        result, _ = pool.refresh(registry, catalogs, definitions)
        old = next(m for m in result["models"] if m["id"] == "luna")["routes"]["vercel"]
        self.assertFalse(old["listed"])
        self.assertEqual(old["model"], "openai/luna")

    def test_empty_catalog_fails_closed(self):
        registry, catalogs, definitions = self.inputs()
        catalogs["vercel"] = []
        with self.assertRaisesRegex(ValueError, "empty vercel"):
            pool.refresh(registry, catalogs, definitions)

    def test_malformed_catalog_rows_fail_closed(self):
        for row in [None, "model", {}, {"id": ""}, {"id": 42}]:
            registry, catalogs, definitions = self.inputs()
            catalogs["vercel"] = [row]
            with self.assertRaisesRegex(ValueError, "invalid or empty vercel"):
                pool.refresh(registry, catalogs, definitions)

    def test_exclusions_aliases_and_unknown_go_identity(self):
        for provider, identifier in [("vercel", "meta/llama"), ("openrouter", "x-ai/grok"),
                                     ("vercel", "spacexai/grok"), ("opencode-go", "omen-alpha"),
                                     ("openrouter", "~vendor/latest"), ("openrouter", "vendor/model:batch")]:
            model = {"id": identifier, "type": "language", "tags": ["tool-use"], "supported_parameters": ["tools"]}
            self.assertFalse(pool.candidate(provider, model, {}))
