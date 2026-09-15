"""Run OpenCode inside an AoE guest; keep provider credentials on the host."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

VERSION = "1.18.30"
SHA256 = "60c92147d0d86ca606dda8a77260d3c87e0ef959eb2d8dbffb34df6d8a64e063"
ASSET = "opencode-linux-x64-baseline.tar.gz"
ROUTES = {
    "opencode-go": ("OPENCODE_API_KEY", "https://opencode.ai/zen/go/v1"),
    "opencode": ("OPENCODE_API_KEY", "https://opencode.ai/zen/v1"),
    "openrouter": ("OPENROUTER_API_KEY", "https://openrouter.ai/api/v1"),
}
MAX_EVENTS = 64 * 1024 * 1024


def go_model(model):
    provider, _, identifier = model.partition("/")
    if provider != "opencode-go":
        raise ValueError(f"preflight requires an explicit OpenCode Go model, got {model!r}")
    registry = json.loads((Path(__file__).resolve().parent.parent / "suites/model-pool.json").read_text())
    if registry.get("schema_version") != 1:
        raise ValueError("unsupported model registry version")
    definitions = [route["definition"] for entry in registry["models"]
                   if (route := entry.get("routes", {}).get("opencode-go"))
                   and route["model"] == identifier and route.get("definition")]
    if len(definitions) != 1 or definitions[0].get("id") != identifier:
        raise ValueError(f"model {model!r} has no pinned definition; update the catalog explicitly (no substitutions)")
    # Eligibility governs new draws, not previously committed runs.
    return identifier, definitions[0]


def preflight(models):
    """Resolve the entire fleet without credentials, model requests, or VMs."""
    definitions = dict(go_model(model) for model in sorted(set(models)))
    if not definitions:
        raise ValueError("preflight needs at least one model")
    response = subprocess.run(["curl", "--fail", "--silent", "--show-error", "--max-time", "20",
                               "https://opencode.ai/zen/go/v1/models"], capture_output=True, text=True, timeout=25)
    if response.returncode:
        raise ValueError(f"cannot check Go model catalog: {response.stderr.strip()}")
    advertised = {entry["id"] for entry in json.loads(response.stdout)["data"]}
    missing = sorted(set(definitions) - advertised)
    if missing:
        raise ValueError(f"OpenCode Go does not advertise: {', '.join(missing)}")
    executable = binary()
    with tempfile.TemporaryDirectory(prefix="aoe-opencode-preflight-") as directory:
        root = Path(directory)
        config = {"enabled_providers": ["opencode-go"], "autoupdate": False, "share": "disabled",
                  "provider": {"opencode-go": {"models": definitions,
                      "options": {"apiKey": "preflight-no-credential", "baseURL": "http://127.0.0.1:1"}}}}
        config_path = root / "opencode.json"
        config_path.write_text(json.dumps(config))
        env = {"PATH": os.environ.get("PATH", ""), "NO_COLOR": "1",
               "XDG_CONFIG_HOME": str(root / "config"), "XDG_DATA_HOME": str(root / "data"),
               "XDG_CACHE_HOME": str(root / "cache"), "OPENCODE_CONFIG": str(config_path),
               "OPENCODE_DISABLE_MODELS_FETCH": "true", "OPENCODE_DISABLE_AUTOUPDATE": "true"}
        result = subprocess.run([str(executable), "models", "opencode-go"], cwd=root, env=env,
                                capture_output=True, text=True, timeout=45)
        if result.returncode:
            raise ValueError(f"pinned OpenCode rejected model configuration: {result.stderr[-4000:]}")
        resolved = set(result.stdout.splitlines())
        for identifier in definitions:
            if f"opencode-go/{identifier}" not in resolved:
                raise ValueError(f"pinned OpenCode cannot resolve opencode-go/{identifier}")
    print(f"OpenCode preflight passed: {', '.join(sorted(definitions))} (catalog/configuration only; account quota not checked)")


def normalize(data, exit_code=None, reboot=False):
    """A truncated final JSONL record is expected when SSH/guest is interrupted."""
    events = []
    lines = data.splitlines(keepends=True)
    for index, line in enumerate(lines):
        try:
            event = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            if index == len(lines) - 1 and not line.endswith(b"\n"):
                break
            raise ValueError("malformed OpenCode event stream") from None
        if not isinstance(event, dict):
            raise ValueError("OpenCode event must be an object")
        events.append(event)
    steps = [e["part"] for e in events if e.get("type") == "step_finish"]
    errors = [e.get("error", {}) for e in events if e.get("type") == "error"]
    usage = {"rounds": len(steps), "tool_calls": sum(e.get("type") == "tool_use" for e in events),
             "input_tokens": None, "output_tokens": None, "cost_microusd": None,
             "resource_units": 1}
    for name, field in [("input_tokens", "input"), ("output_tokens", "output")]:
        values = [s.get("tokens", {}).get(field) for s in steps]
        if values and all(type(v) is int and v >= 0 for v in values):
            usage[name] = sum(values)
    status, summary = "running", "OpenCode is running"
    if errors:
        error = errors[-1]
        details = error.get("data", {})
        code = details.get("statusCode")
        name = error.get("name", "")
        summary = details.get("message") or error.get("message") or name or "OpenCode error"
        unavailable = (code in (401, 402, 403, 404, 408, 429) or
                       isinstance(code, int) and code >= 500 or
                       name in ("ProviderAuthError", "ProviderModelNotFoundError"))
        if unavailable:
            status = "unavailable"
        elif name == "ContextOverflowError":
            status = "failed"
        else:
            status = "harness_error"
    elif exit_code is not None:
        if reboot and exit_code == 255:
            status, summary = "interrupted", "agent session was interrupted by the referee's host reboot"
        elif exit_code == 255:
            status, summary = "harness_error", "OpenCode SSH disconnected; cause unknown; partial evidence retained"
        elif exit_code == 137:
            status, summary = "harness_error", "OpenCode was killed (exit 137); possible OOM or external SIGKILL; inspect VM console"
        elif exit_code in (130, 143):
            status, summary = "interrupted", "OpenCode adapter was interrupted"
        elif exit_code == 0 and steps and steps[-1].get("reason") in ("stop", "end_turn"):
            status = "completed"
            summary = "\n".join(e.get("part", {}).get("text", "") for e in events if e.get("type") == "text") or "OpenCode completed"
        else:
            status = "failed" if steps else "harness_error"
            summary = f"OpenCode exited with status {exit_code} without a completed turn"
    origin = min((e.get("timestamp", 0) for e in events), default=0)
    tools = []
    for event in events:
        if event.get("type") != "tool_use":
            continue
        part = event.get("part", {})
        state = part.get("state", {})
        timing = state.get("time", {})
        start = timing.get("start", event.get("timestamp", origin))
        tools.append({"id": part.get("callID", part.get("id", "")), "name": part.get("tool", "unknown"),
                      "input": state.get("input", {}), "output": state.get("output", state.get("error", "")),
                      "is_error": state.get("status") == "error" or state.get("metadata", {}).get("exit", 0) != 0,
                      "read_only": part.get("tool") in ("read", "glob", "grep", "list", "webfetch"),
                      "started_after_ms": max(0, start - origin),
                      "duration_ms": max(0, timing.get("end", start) - start)})
    estimates = [s.get("cost") for s in steps]
    estimate = sum(estimates) if estimates and all(type(v) in (int, float) and v >= 0 for v in estimates) else None
    transcript = {"schema_version": 2, "harness": "opencode", "tool_trace": tools,
                  "messages": [{"role": "assistant", "content": [{"type": "text", "text": e.get("part", {}).get("text", "")}]} for e in events if e.get("type") == "text"],
                  "usage": {"input_tokens": usage["input_tokens"], "output_tokens": usage["output_tokens"],
                            "cost_usd": None, "estimated_cost_usd": estimate},
                  "outcome": {"status": status, "message": summary}}
    return status, str(summary), usage, transcript


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".partial")
    temporary.write_text(json.dumps(value, ensure_ascii=False))
    temporary.replace(path)


def binary():
    override = os.environ.get("AOE_OPENCODE_BINARY")
    if override:
        path = Path(override).resolve()
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError("AOE_OPENCODE_BINARY must be an executable Linux x86_64 binary")
        return path
    root = Path.home() / ".cache/agents-of-empires/opencode" / VERSION
    root.mkdir(parents=True, exist_ok=True)
    archive = root / ASSET
    if not archive.is_file() or hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        with urllib.request.urlopen(f"https://github.com/anomalyco/opencode/releases/download/v{VERSION}/{ASSET}", timeout=30) as response:
            data = response.read(512 * 1024 * 1024 + 1)
        if hashlib.sha256(data).hexdigest() != SHA256:
            raise ValueError("pinned OpenCode archive checksum mismatch")
        archive.write_bytes(data)
    # Extract only the executable, not arbitrary archive paths or links.
    with tarfile.open(archive) as bundle:
        member = next(m for m in bundle.getmembers() if m.isfile() and Path(m.name).name == "opencode")
        data = bundle.extractfile(member).read()
    with tempfile.NamedTemporaryFile(dir=root, delete=False) as target:
        target.write(data)
        path = Path(target.name)
    path.chmod(0o700)
    path.replace(root / "opencode")
    return root / "opencode"


def main():
    names = ["AGENT_ID", "TERRITORY_ID", "TERRITORY_HOST", "SSH_PORT", "MODEL", "REASONING_EFFORT",
             "INSTRUCTION_FILE", "RESULT_FILE", "USAGE_FILE", "CREDENTIAL_FILE"]
    values = {name: os.environ["AOE_" + name] for name in names}
    root = Path(values["RESULT_FILE"]).parent
    root.mkdir(parents=True, exist_ok=True)
    children = []
    secret = ""
    usage = {"resource_units": 1}
    status, summary = "harness_error", "OpenCode setup did not complete"
    transcript_path = root / "transcript.json"
    model = values["MODEL"]
    identity = {"schema_version": 1, "agent": values["AGENT_ID"], "territory": values["TERRITORY_ID"]}

    def stop(signum, _frame):
        raise InterruptedError(f"signal {signum}")

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        provider, sep, _model_id = model.partition("/")
        if not sep or not _model_id or provider not in ROUTES:
            raise ValueError("model must use opencode-go/, opencode/, or openrouter/ provider prefix")
        key_name, upstream = ROUTES[provider]
        # Source controller-owned shell credentials, never copy them into a VM.
        loaded = subprocess.run(["bash", "-c", 'set -a; source "$1"; exec env -0', "bash", values["CREDENTIAL_FILE"]],
                                check=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10)
        env = dict(item.decode().split("=", 1) for item in loaded.stdout.split(b"\0") if b"=" in item)
        secret = env.get(key_name, "")
        if not secret or not env.get("AOE_SSH_PASSWORD"):
            raise ValueError(f"credential file requires {key_name} and AOE_SSH_PASSWORD")
        executable = binary()
        repo = Path(__file__).resolve().parent.parent
        proxy = Path(os.environ.get("AOE_OPENCODE_PROXY", str(repo.parent / "replaybook/integrations/host/openrouter_proxy.py")))
        if not proxy.is_file():
            raise ValueError("credential proxy missing; set AOE_OPENCODE_PROXY")
        env.update(SSH_ASKPASS=str(repo / "adapters/ssh-askpass.sh"), SSH_ASKPASS_REQUIRE="force", DISPLAY=":0")
        port = int(values["SSH_PORT"])
        options = ["-o", "ConnectTimeout=5", "-o", "LogLevel=ERROR", "-o", "PreferredAuthentications=password",
                   "-o", "PubkeyAuthentication=no", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]
        host = "root@" + values["TERRITORY_HOST"]
        ssh = ["ssh", "-p", str(port), *options, host]
        scp = ["scp", "-q", "-P", str(port), *options]
        remote = "/var/tmp/aoe-opencode-" + hashlib.sha256(values["AGENT_ID"].encode()).hexdigest()[:16]

        def run(args, timeout=20):
            return subprocess.run(args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=timeout)

        def setup(args):
            for attempt in range(3):
                if run(args).returncode == 0:
                    return
                time.sleep(attempt + 1)
            raise ValueError("OpenCode SSH setup failed after three attempts")

        ready = root / "opencode-proxy.port"
        ready.unlink(missing_ok=True)
        proxy_env = dict(env, REPLAYBOOK_OPENAI_API_KEY=secret)
        with (root / "proxy.log").open("wb") as log:
            process = subprocess.Popen([sys.executable, str(proxy), "--port", "0", "--ready-file", str(ready), "--upstream", upstream], env=proxy_env, stdout=log, stderr=log)
        children.append(process)
        until = time.monotonic() + 30
        while not ready.is_file():
            if process.poll() is not None or time.monotonic() > until:
                raise ValueError("OpenCode credential proxy failed to start")
            time.sleep(0.1)
        local_port = int(ready.read_text())
        remote_port = 18000 + port % 1000
        tunnel = subprocess.Popen([*ssh, "-N", "-o", "ExitOnForwardFailure=yes", "-R", f"127.0.0.1:{remote_port}:127.0.0.1:{local_port}"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        children.append(tunnel)
        time.sleep(0.3)
        if tunnel.poll() is not None:
            raise ValueError("OpenCode credential tunnel failed")
        setup([*ssh, f"install -d -m 0700 {shlex.quote(remote)}"])
        setup([*scp, str(executable), f"{host}:{remote}/opencode"])
        setup([*ssh, f"chmod 700 {remote}/opencode && {remote}/opencode --version"])
        setup([*scp, values["INSTRUCTION_FILE"], f"{host}:{remote}/instruction.md"])
        config = {"$schema": "https://opencode.ai/config.json", "enabled_providers": [provider], "model": model,
                  "autoupdate": False, "share": "disabled", "permission": "allow",
                  "provider": {provider: {"options": {"apiKey": "arena-proxy-placeholder", "baseURL": f"http://127.0.0.1:{remote_port}"}}}}
        if provider == "opencode-go":
            identifier, definition = go_model(model)
            config["provider"][provider]["models"] = {identifier: definition}
        config_path = root / "opencode-config.json"
        atomic_json(config_path, config)
        setup([*scp, str(config_path), f"{host}:{remote}/opencode.json"])
        args = [f"{remote}/opencode", "--print-logs", "--log-level", "DEBUG", "run", "--pure", "--format", "json", "--auto", "--dir", "/root", "--model", model]
        if values["REASONING_EFFORT"] and values["REASONING_EFFORT"] != "default":
            args.extend(["--variant", values["REASONING_EFFORT"]])
        artifacts = json.loads(os.environ.get("AOE_PLAYER_ARTIFACTS_JSON", "[]"))
        if not isinstance(artifacts, list) or not all(isinstance(p, str) for p in artifacts):
            raise ValueError("invalid player artifact list")
        for index, artifact in enumerate(artifacts):
            extension = Path(artifact).suffix.lower()
            if extension not in (".png", ".jpg", ".jpeg", ".gif", ".webp", ".pdf", ".txt"):
                extension = ".bin"
            destination = f"{remote}/artifact-{index}{extension}"
            setup([*scp, artifact, f"{host}:{destination}"])
            args.extend(["--file", destination])
        command = (f"chmod 700 {remote}/opencode; "
                   f"export XDG_CONFIG_HOME={remote}/config XDG_DATA_HOME={remote}/data XDG_CACHE_HOME={remote}/cache; "
                   f"export OPENCODE_CONFIG={remote}/opencode.json OPENCODE_DISABLE_AUTOUPDATE=true OPENCODE_DISABLE_MODELS_FETCH=true; "
                   f"{shlex.join(args)} -- \"$(cat {remote}/instruction.md)\" > {remote}/events.jsonl")
        with (root / "stderr.log").open("ab") as log:
            os.chmod(root / "stderr.log", 0o600)
            agent = subprocess.Popen([*ssh, command], env=env, stdout=subprocess.DEVNULL, stderr=log)
        children.append(agent)
        raw = root / "opencode-events.jsonl"

        def collect(code=None, reboot=False):
            nonlocal status, summary, usage
            partial = root / "opencode-events.partial"
            try:
                copied = run([*scp, f"{host}:{remote}/events.jsonl", str(partial)], timeout=8).returncode == 0
            except subprocess.TimeoutExpired:
                copied = False
            if copied:
                if partial.stat().st_size > MAX_EVENTS:
                    raise ValueError("OpenCode event stream exceeds 64 MiB limit")
                normalize(partial.read_bytes())
                partial.replace(raw)
            data = raw.read_bytes() if raw.exists() else b""
            status, summary, usage, transcript = normalize(data, code, reboot)
            transcript["model"] = model
            # Exact credential redaction also covers provider error echoes.
            transcript = json.loads(json.dumps(transcript).replace(secret, "[redacted]"))
            atomic_json(transcript_path, transcript)
            atomic_json(Path(values["USAGE_FILE"]), dict(identity, usage=usage))

        while agent.poll() is None:
            collect()
            time.sleep(2)
        reboot = agent.returncode == 255 and (root / "referee-reboot").is_file()
        if reboot:
            until = time.monotonic() + 60
            while time.monotonic() < until and run([*ssh, "true"]).returncode != 0:
                time.sleep(1)
        collect(agent.returncode, reboot)
    except InterruptedError:
        status, summary = "interrupted", "OpenCode adapter interrupted; retained latest usage checkpoint"
    except Exception as error:
        status, summary = "harness_error", f"OpenCode adapter: {type(error).__name__}: {error}"
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        if secret:
            summary = summary.replace(secret, "[redacted]")
            diagnostic = root / "stderr.log"
            if diagnostic.exists():
                diagnostic.write_bytes(diagnostic.read_bytes().replace(secret.encode(), b"[redacted]"))
        atomic_json(Path(values["RESULT_FILE"]), dict(identity, status=status, summary=summary, usage=usage,
                    transcript=str(transcript_path) if transcript_path.exists() else None))
    return 0 if status == "completed" else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--preflight":
        try:
            preflight(sys.argv[2:])
        except Exception as error:
            print(f"OpenCode preflight failed: {error}", file=sys.stderr)
            sys.exit(2)
    else:
        sys.exit(main())
