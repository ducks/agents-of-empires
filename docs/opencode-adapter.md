# OpenCode adapter

`adapters/opencode.sh` runs OpenCode **inside** the disposable guest, with the
same instructions, referee, budgets, and result contract as Claux.

The default Linux x86_64 baseline release is **v1.18.30**, verified against
its official archive SHA-256. Guests need `programs.nix-ld`, already enabled
in the bundled arenas. Set `AOE_OPENCODE_BINARY` to an absolute executable
path to use a locally built Linux binary. The checkout at `~/dev/opencode`
is not implicitly built or modified.

## Configuration

Use explicit OpenCode `provider/model` identifiers in fleet entries:

| Prefix | Route | Host credential |
| --- | --- | --- |
| `opencode-go/` | OpenCode Go | `OPENCODE_API_KEY` |
| `opencode/` | OpenCode Zen | `OPENCODE_API_KEY` |
| `openrouter/` | OpenRouter through OpenCode | `OPENROUTER_API_KEY` |

OpenRouter identifiers include both namespaces, for example
`openrouter/moonshotai/kimi-k3`. Check model availability before drawing;
the adapter never silently substitutes a provider or model.
Set `adapter = "opencode"` and map it with
`--adapter opencode=adapters/opencode.sh`.

For the bundled infrastructure pool, in Bash inside `nix-shell`:

```sh
# Reuses your exported OPENCODE_API_KEY. Never print it.
credentials="$(scripts/prepare-opencode-credentials.sh)"
credential_args=()
for file in "$credentials"/*.env; do
  credential_args+=(--credential "$(basename "$file" .env)=$file")
done
# Only when inference is requested, against an OpenCode-configured draw:
cargo run --release --bin agents-of-empires -- season run PATH_TO_DRAW \
  --adapter opencode=adapters/opencode.sh "${credential_args[@]}"
```

For OpenRouter-backed OpenCode, use `prepare-infra-core-credentials.sh`.
Custom arenas need their own territory/password credential files.
The host proxy defaults to
`../replaybook/integrations/host/openrouter_proxy.py`; override its location
with `AOE_OPENCODE_PROXY`. Only a placeholder key and loopback URL enter the
guest. Real credentials remain on the host. Sharing, automatic updates, and
external plugins are disabled; reasoning effort maps to `--variant`.

## Evidence and failures

- `opencode-events.jsonl` retains native streamed events locally.
- `transcript.json` contains observable text and tool activity, excluding
  private reasoning. `usage.json` is an atomic cumulative checkpoint.
- A truncated final event or SCP timeout does not erase the last checkpoint.
- Structured rate limits, authentication errors, and unavailable providers
  are not player failures, even when OpenCode exits zero. Context exhaustion
  remains a player outcome. Unexplained SSH loss remains a harness failure;
  a marked referee reboot is an interruption.
- OpenCode's per-step `cost` is a catalog estimate, not billed cost. We keep
  `estimated_cost_usd` in the transcript and leave `cost_microusd` unknown.
  Season totals do not yet display unknown-cost coverage: a zero aggregate
  must not be described as a free OpenCode run.

Offline tests run in `make lint`, or separately with
`python3 -m unittest discover -s adapters -p 'test_opencode*.py'`.
These test normalization and the full wrapper using fake SSH/SCP/proxy
processes. Live provider tests are never run by default.

References: [CLI](https://opencode.ai/docs/cli/),
[configuration](https://opencode.ai/docs/config/),
[pinned release](https://github.com/anomalyco/opencode/releases/tag/v1.18.30).
