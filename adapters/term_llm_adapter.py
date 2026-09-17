"""Pinned term-llm guest adapter. Provider credentials stay in the host proxy."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

VERSION = "0.9.48"
SHA256 = "da0e138a65e14864af66252b7282980da4597d3a9ffb1695a4104acb82186c52"
ASSET = "term-llm_0.9.48_linux_amd64.tar.gz"
ROUTES = {
    "openrouter": ("OPENROUTER_API_KEY", "https://openrouter.ai/api/v1"),
    "vercel": ("AI_GATEWAY_API_KEY", "https://ai-gateway.vercel.sh/v1"),
}
MAX_EVENTS = 64 * 1024 * 1024


def guest_config(model, effort, port):
    if effort not in ("default", "none", "minimal", "low", "medium", "high", "xhigh", "max"):
        raise ValueError("unsupported reasoning effort")
    definition = {"id": model, "alias": "entrant"}
    if effort != "default":
        definition.update(reasoning_efforts=[effort], default_reasoning_effort=effort)
    return {"default_provider": "arena", "providers": {"arena": {
        "type": "openai_compatible", "base_url": f"http://127.0.0.1:{port}",
        "api_key": "arena-proxy-placeholder", "model": "entrant", "models": [definition]}},
        "sessions": {"enabled": False}}


def normalize(data, exit_code=None, reboot=False):
    """Consume checkpoints without counting final cumulative stats twice."""
    events = []
    lines = data.splitlines(keepends=True)
    for index, line in enumerate(lines):
        try:
            event = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            if index == len(lines) - 1 and not line.endswith(b"\n"):
                break
            raise ValueError("malformed term-llm event stream") from None
        if not isinstance(event, dict) or not isinstance(event.get("type"), str):
            raise ValueError("term-llm event must be a typed object")
        events.append(event)
    stats = [e for e in events if e["type"] == "stats"]
    deltas = [e for e in events if e["type"] in ("usage", "guardian.review")]
    usage = {"rounds": None, "tool_calls": sum(e["type"] == "tool.started" for e in events),
             "input_tokens": None, "output_tokens": None, "cost_microusd": None, "resource_units": 1}
    for name in ("input_tokens", "output_tokens"):
        values = [stats[-1].get(name)] if stats else [e.get(name) for e in deltas]
        if values and all(type(v) is int and v >= 0 for v in values):
            usage[name] = sum(values)
    if usage["input_tokens"] is not None:
        sources = [stats[-1]] if stats else deltas
        cached = [e.get(k, 0) for e in sources for k in ("cached_input_tokens", "cache_write_tokens")]
        usage["input_tokens"] = (usage["input_tokens"] + sum(cached)
                                 if all(type(v) is int and v >= 0 for v in cached) else None)
    if stats and type(stats[-1].get("llm_calls")) is int and stats[-1]["llm_calls"] >= 0:
        usage["rounds"] = stats[-1]["llm_calls"]
    errors = [str(e.get("message", "term-llm error")) for e in events if e["type"] == "error"]
    status, summary = "running", "term-llm is running"
    if exit_code in (130, 143, -2, -15) or reboot and exit_code == 255:
        status, summary = "interrupted", "term-llm session interrupted by controller or referee"
    elif exit_code in (255, 137):
        status, summary = "harness_error", f"term-llm transport/process ended with status {exit_code}; inspect VM evidence"
    elif errors:
        summary = errors[-1]
        if re.search(r"(?:HTTP|status(?: code)?|API error)[\s:(]*(401|402|403|404|408|429|5\d\d)\b|unauthorized|rate limit|quota|model not found", summary, re.I):
            status = "unavailable"
        elif re.search(r"cancelled|canceled|deadline exceeded", summary, re.I):
            status = "interrupted"
        elif re.search(r"max.?turns|maximum.*turn|context.*(limit|length|exceed)", summary, re.I):
            status = "failed"
        else:
            status = "harness_error"
    elif exit_code is not None:
        if exit_code == 0 and stats and any(e["type"] == "session.started" for e in events) and events[-1]["type"] == "done":
            status, summary = "completed", "term-llm finished its agent turn; deployment is judged by the referee"
        else:
            status, summary = "harness_error", f"term-llm exited {exit_code} without a complete event stream"
    def timestamp(event):
        try:
            return datetime.datetime.fromisoformat(event["ts"].replace("Z", "+00:00")).timestamp() * 1000
        except (KeyError, TypeError, ValueError, AttributeError):
            return 0
    origin = timestamp(events[0]) if events else 0
    trace, calls = [], {}
    for event in events:
        if event["type"] == "tool.started":
            tool = {"id": event.get("call_id", ""), "name": event.get("name", "unknown"),
                    "input": event.get("args", {}), "output": "", "is_error": False,
                    "read_only": event.get("name") in ("read_file", "grep", "glob"),
                    "started_after_ms": max(0, int(timestamp(event) - origin)), "duration_ms": 0}
            calls[tool["id"]] = (tool, timestamp(event))
            trace.append(tool)
        elif event["type"] == "tool.completed" and event.get("call_id") in calls:
            tool, start = calls[event["call_id"]]
            tool.update(is_error=event.get("success") is not True,
                        duration_ms=max(0, int(timestamp(event) - start)),
                        output="[Completion summary; full tool output unavailable] " + str(event.get("info", "")))
    text = "".join(str(e.get("text", "")) for e in events if e["type"] == "text.delta")
    transcript = {"schema_version": 2, "harness": "term-llm", "harness_version": VERSION,
                  "tool_trace": trace, "messages": [{"role": "assistant", "content": [{"type": "text", "text": text}]}],
                  "usage": {"input_tokens": usage["input_tokens"], "output_tokens": usage["output_tokens"], "cost_usd": None},
                  "outcome": {"status": status, "message": summary}}
    return status, summary, usage, transcript


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".partial")
    temporary.write_text(json.dumps(value, ensure_ascii=False))
    temporary.chmod(0o600)
    temporary.replace(path)


def binary():
    root = Path.home() / ".cache/agents-of-empires/term-llm" / VERSION
    root.mkdir(parents=True, exist_ok=True)
    archive = root / ASSET
    if not archive.is_file() or hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        with urllib.request.urlopen(f"https://github.com/SamSaffron/term-llm/releases/download/v{VERSION}/{ASSET}", timeout=30) as response:
            data = response.read(512 * 1024 * 1024 + 1)
        if hashlib.sha256(data).hexdigest() != SHA256:
            raise ValueError("pinned term-llm archive checksum mismatch")
        with tempfile.NamedTemporaryFile(dir=root, delete=False) as target:
            target.write(data)
            downloaded = Path(target.name)
        downloaded.replace(archive)
    # Extract only the executable, not arbitrary archive paths or links.
    with tarfile.open(archive) as bundle:
        members = [m for m in bundle.getmembers() if m.isfile() and m.name in ("term-llm", "./term-llm")]
        if len(members) != 1 or members[0].size > 256 * 1024 * 1024:
            raise ValueError("invalid term-llm archive executable")
        member = members[0]
        data = bundle.extractfile(member).read()
    with tempfile.NamedTemporaryFile(dir=root, delete=False) as target:
        target.write(data)
        path = Path(target.name)
    path.chmod(0o700)
    path.replace(root / "term-llm")
    return root / "term-llm"


def main():
    names = ["AGENT_ID", "TERRITORY_ID", "TERRITORY_HOST", "SSH_PORT", "MODEL", "REASONING_EFFORT",
             "INSTRUCTION_FILE", "RESULT_FILE", "USAGE_FILE", "CREDENTIAL_FILE"]
    values = {name: os.environ["AOE_" + name] for name in names}
    root = Path(values["RESULT_FILE"]).parent
    root.mkdir(parents=True, exist_ok=True)
    children = []
    secret = ""
    usage = {"resource_units": 1}
    status, summary = "harness_error", "term-llm setup did not complete"
    transcript_path = root / "transcript.json"
    model = values["MODEL"]
    identity = {"schema_version": 1, "agent": values["AGENT_ID"], "territory": values["TERRITORY_ID"]}

    def stop(signum, _frame):
        raise InterruptedError(f"signal {signum}")

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        provider, sep, _model_id = model.partition("/")
        if not sep or provider not in ROUTES or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", _model_id):
            raise ValueError("model must use openrouter/vendor/model or vercel/vendor/model")
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
        proxy = Path(os.environ.get("AOE_TERM_LLM_PROXY", str(repo.parent / "replaybook/integrations/host/openrouter_proxy.py")))
        if not proxy.is_file():
            raise ValueError("credential proxy missing; set AOE_TERM_LLM_PROXY")
        env.update(SSH_ASKPASS=str(repo / "adapters/ssh-askpass.sh"), SSH_ASKPASS_REQUIRE="force", DISPLAY=":0")
        port = int(values["SSH_PORT"])
        options = ["-o", "ConnectTimeout=5", "-o", "LogLevel=ERROR", "-o", "PreferredAuthentications=password",
                   "-o", "PubkeyAuthentication=no", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]
        host = "root@" + values["TERRITORY_HOST"]
        ssh = ["ssh", "-p", str(port), *options, host]
        scp = ["scp", "-q", "-P", str(port), *options]
        remote = "/var/tmp/aoe-term-llm-" + hashlib.sha256(values["AGENT_ID"].encode()).hexdigest()[:16]

        def run(args, timeout=20):
            return subprocess.run(args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=timeout)

        def setup(args):
            for attempt in range(3):
                if run(args).returncode == 0:
                    return
                time.sleep(attempt + 1)
            raise ValueError("term-llm SSH setup failed after three attempts")

        ready = root / "term-llm-proxy.port"
        ready.unlink(missing_ok=True)
        proxy_env = dict(env, REPLAYBOOK_OPENAI_API_KEY=secret)
        with (root / "proxy.log").open("wb") as log:
            process = subprocess.Popen([sys.executable, str(proxy), "--port", "0", "--ready-file", str(ready), "--upstream", upstream], env=proxy_env, stdout=log, stderr=log)
        children.append(process)
        until = time.monotonic() + 30
        while not ready.is_file():
            if process.poll() is not None or time.monotonic() > until:
                raise ValueError("term-llm credential proxy failed to start")
            time.sleep(0.1)
        local_port = int(ready.read_text())
        remote_port = 18000 + port % 1000
        tunnel = subprocess.Popen([*ssh, "-N", "-o", "ExitOnForwardFailure=yes", "-R", f"127.0.0.1:{remote_port}:127.0.0.1:{local_port}"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        children.append(tunnel)
        time.sleep(0.3)
        if tunnel.poll() is not None:
            raise ValueError("term-llm credential tunnel failed")
        setup([*ssh, f"install -d -m 0700 {shlex.quote(remote)}"])
        setup([*scp, str(executable), f"{host}:{remote}/term-llm"])
        setup([*ssh, f"chmod 700 {remote}/term-llm && {remote}/term-llm version"])
        setup([*scp, values["INSTRUCTION_FILE"], f"{host}:{remote}/instruction.md"])
        config = guest_config(_model_id, values["REASONING_EFFORT"], remote_port)
        config_path = root / "term-llm-config.yaml"
        # JSON is a YAML subset; avoid a host YAML dependency.
        atomic_json(config_path, config)
        setup([*ssh, f"install -d -m 0700 {remote}/config/term-llm"])
        setup([*scp, str(config_path), f"{host}:{remote}/config/term-llm/config.yaml"])
        args = [f"{remote}/term-llm", "ask", "--json", "--no-session", "--provider", "arena:entrant",
                "--approval", "yolo", "--tools", "read_file,write_file,edit_file,shell,grep,glob",
                "--skills", "none", "--no-search", "--max-turns", "200"]
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
        command = (f"cd /root && "
                   f"export XDG_CONFIG_HOME={remote}/config XDG_DATA_HOME={remote}/data XDG_CACHE_HOME={remote}/cache; "
                   f"{shlex.join(args)} -- \"$(cat {remote}/instruction.md)\" > {remote}/events.jsonl")
        with (root / "stderr.log").open("ab") as log:
            os.chmod(root / "stderr.log", 0o600)
            agent = subprocess.Popen([*ssh, command], env=env, stdout=subprocess.DEVNULL, stderr=log)
        children.append(agent)
        raw = root / "term-llm-events.jsonl"

        def collect(code=None, reboot=False):
            nonlocal status, summary, usage
            partial = root / "term-llm-events.partial"
            try:
                copied = run([*scp, f"{host}:{remote}/events.jsonl", str(partial)], timeout=8).returncode == 0
            except subprocess.TimeoutExpired:
                copied = False
            if copied:
                if partial.stat().st_size > MAX_EVENTS:
                    raise ValueError("term-llm event stream exceeds 64 MiB limit")
                data = partial.read_bytes().replace(secret.encode(), b"[redacted]")
                normalize(data)
                partial.write_bytes(data)
                partial.chmod(0o600)
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
        status, summary = "interrupted", "term-llm adapter interrupted; retained latest usage checkpoint"
    except Exception as error:
        status, summary = "harness_error", f"term-llm adapter: {type(error).__name__}: {error}"
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
    if sys.argv[1:] == ["--preflight"]:
        executable = binary()
        with tempfile.TemporaryDirectory(prefix="aoe-term-llm-preflight-") as directory:
            env = {"PATH": os.environ.get("PATH", ""), "XDG_CONFIG_HOME": directory,
                   "XDG_DATA_HOME": directory, "XDG_CACHE_HOME": directory}
            subprocess.run([str(executable), "version"], env=env, cwd=directory, check=True, timeout=15)
            subprocess.run([str(executable), "ask", "--help"], env=env, cwd=directory, check=True, timeout=15)
    else:
        sys.exit(main())
