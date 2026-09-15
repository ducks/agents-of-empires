#!/usr/bin/env python3
"""Refresh public catalog metadata; never enable models or run inference."""
import argparse
import copy
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
URLS = {
    "openrouter": "https://openrouter.ai/api/v1/models",
    "vercel": "https://ai-gateway.vercel.sh/v1/models",
    "opencode-go": "https://opencode.ai/zen/go/v1/models",
}
DEFINITION_KEYS = {"id", "name", "family", "attachment", "reasoning", "temperature",
                   "tool_call", "interleaved", "limit", "cost", "modalities"}


def fetch(url):
    response = subprocess.run(["curl", "--fail", "--silent", "--show-error",
                               "--max-time", "30", url], capture_output=True, text=True, timeout=35)
    if response.returncode:
        raise ValueError(f"catalog fetch failed for {url}: {response.stderr.strip()}")
    return json.loads(response.stdout)


def family(provider, identifier):
    vendor = identifier.split("/")[0]
    aliases = {"z-ai": "zai", "alibaba": "qwen", "mistralai": "mistral",
               "bytedance-seed": "bytedance", "meta-llama": "meta", "x-ai": "xai", "spacexai": "xai"}
    if provider != "opencode-go":
        return aliases.get(vendor, vendor)
    for prefix, name in [("gpt-", "openai"), ("glm-", "zai"), ("kimi-", "moonshotai"),
                         ("mimo-", "xiaomi"), ("hy", "tencent"), ("longcat-", "meituan"),
                         ("qwen", "qwen"), ("minimax-", "minimax"), ("deepseek", "deepseek"),
                         ("grok-", "xai"), ("muse-", "meta")]:
        if identifier.startswith(prefix):
            return name
    return "unknown"


def candidate(provider, model, definitions):
    identifier = model["id"]
    if family(provider, identifier) in {"meta", "xai", "unknown"}:
        return False
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", identifier):
        return False  # Floating aliases and :batch/:free variants aren't new entrants.
    if provider == "openrouter":
        return "tools" in model.get("supported_parameters", []) and not model.get("alias_target")
    if provider == "vercel":
        return model.get("type") == "language" and "tool-use" in model.get("tags", [])
    return definitions.get(identifier, {}).get("tool_call") is True


def refresh(registry, catalogs, definitions):
    if registry.get("schema_version") != 1:
        raise ValueError("unsupported model registry version")
    # Fail closed: a partial/empty response must not delist the entire fleet.
    for provider in URLS:
        rows = catalogs.get(provider)
        if not isinstance(rows, list) or not rows or any(
            not isinstance(m, dict) or not isinstance(m.get("id"), str) or not m["id"]
            for m in rows
        ):
            raise ValueError(f"invalid or empty {provider} catalog")
        if len({m["id"] for m in rows}) != len(rows):
            raise ValueError(f"duplicate IDs in {provider} catalog")
    if not isinstance(definitions, dict) or not definitions:
        raise ValueError("empty OpenCode definitions")
    result = copy.deepcopy(registry)
    models = result["models"]
    known = {}
    for model in models:
        for provider, route in model["routes"].items():
            key = (provider, route["model"])
            if key in known:
                raise ValueError(f"duplicate registry route {key}")
            known[key] = model
            route["listed"] = False
    by_id = {m["id"]: m for m in models}
    if len(by_id) != len(models):
        raise ValueError("duplicate registry model IDs")
    changes = []
    for provider, rows in catalogs.items():
        for source in rows:
            identifier = source["id"]
            owner = known.get((provider, identifier))
            if owner is None:
                if not candidate(provider, source, definitions):
                    continue
                # Only group a new route when family and exact model suffix match.
                # Provider aliases with different suffixes require human review.
                vendor = family(provider, identifier)
                suffix = identifier.split("/")[-1]
                matches = [m for m in models if m["family"] == vendor and provider not in m["routes"]
                           and any(r["model"].split("/")[-1] == suffix for r in m["routes"].values())]
                owner = matches[0] if len(matches) == 1 else None
                if owner is None:
                    stable_id = re.sub(r"[^A-Za-z0-9._-]", "-", f"{vendor}-{suffix}")
                    if stable_id in by_id:
                        stable_id += "-" + hashlib.sha256(f"{provider}/{identifier}".encode()).hexdigest()[:8]
                    if stable_id in by_id:
                        raise ValueError(f"model identity collision: {stable_id}")
                    owner = {"id": stable_id, "name": source.get("name", identifier), "family": vendor, "routes": {}}
                    models.append(owner)
                    by_id[stable_id] = owner
                owner["routes"][provider] = {"model": identifier, "reasoning_effort": "default",
                    "status": "disabled", "listed": True, "note": "New catalog discovery: review identity, cost, reasoning, and access before enabling.", "pricing": {}}
                changes.append(f"new disabled route: {owner['id']} / {provider} / {identifier}")
            route = owner["routes"][provider]
            route["listed"] = True
            pricing = source.get("pricing", {})
            if route.get("pricing", {}) != pricing:
                changes.append(f"price metadata changed: {owner['id']} / {provider}")
            route["pricing"] = pricing
            if provider == "opencode-go" and identifier in definitions:
                definition = {k: v for k, v in definitions[identifier].items() if k in DEFINITION_KEYS and v is not None}
                if definition.get("id") != identifier:
                    raise ValueError(f"definition identity mismatch: {identifier}")
                if route.get("definition") != definition:
                    changes.append(f"client definition changed: {owner['id']} / {provider}")
                route["definition"] = definition
    for owner in models:
        for provider, route in owner["routes"].items():
            if not route["listed"]:
                changes.append(f"not listed (retained, excluded from new draws): {owner['id']} / {provider}")
    result["models"].sort(key=lambda m: m["id"])
    return result, changes


def atomic_write(path, content):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
            os.chmod(temporary, 0o644)
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--write", action="store_true", help="apply reviewed metadata/discoveries and update the launcher's registry checksum")
    mode.add_argument("--check", action="store_true", help="exit 1 when metadata differs; do not write")
    args = parser.parse_args()
    registry_path = ROOT / "suites/model-pool.json"
    original = json.loads(registry_path.read_text())
    catalogs = {p: fetch(url)["data"] for p, url in URLS.items()}
    definitions = fetch("https://models.dev/api.json")["opencode-go"]["models"]
    result, changes = refresh(original, catalogs, definitions)
    for change in changes:
        print(change)
    changed = result != original
    if args.write:
        result["checked_at"] = datetime.date.today().isoformat()
        content = json.dumps(result, indent=2, ensure_ascii=False) + "\n"
        checksum = hashlib.sha256(content.encode()).hexdigest()
        launcher = ROOT / "adapters/opencode.sh"
        old = launcher.read_text()
        updated, count = re.subn(r'(\[\[ "\$catalog_hash" == ")[a-f0-9]{64}(" \]\])', lambda m: m[1] + checksum + m[2], old)
        if count != 1:
            raise ValueError("cannot locate registry checksum; no files written")
        atomic_write(registry_path, content)
        atomic_write(launcher, updated)
        os.chmod(launcher, 0o755)
        print("Updated registry and launcher checksum. Review git diff and run make lint. No routes were enabled.")
    else:
        print(f"{'Changes available' if changed else 'Up to date'}; dry run, no files written. Use --write to apply.")
    return 1 if args.check and changed else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, OSError, subprocess.TimeoutExpired) as error:
        raise SystemExit(f"Registry refresh failed: {error}") from error
