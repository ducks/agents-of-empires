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
- Internal OpenCode debug logs stream to local `stderr.log` (mode 0600),
  including server diagnostics otherwise hidden behind generic errors. The
  controller credential is redacted at adapter shutdown. These diagnostic
  logs are private and are not included in generated public reports.
- `transcript.json` contains observable text and tool activity, excluding
  private reasoning. `usage.json` is an atomic cumulative checkpoint.
- A truncated final event or SCP timeout does not erase the last checkpoint.
- Structured rate limits, authentication errors, and unavailable providers
  are not player failures, even when OpenCode exits zero. Context exhaustion
  remains a player outcome. Unexplained SSH loss remains a harness failure;
  a marked referee reboot is an interruption.
- OpenCode's per-step `cost` is a catalog estimate, not billed cost. We keep
  `estimated_cost_usd` in the transcript and leave `cost_microusd` unknown.
  Go model rows display `subscription`, and mixed aggregate totals explicitly
  mark subscription usage as excluded from recorded dollar spend. This is a
  route label, not confirmation of account billing: Go limits and optional
  overage still apply. Zen/OpenRouter unknown-cost coverage remains a limitation.

## OpenCode Go Cup

Before booting any VM, Go matches and tournaments run the adapter's host
preflight over all requested Go entrants. It checks the public Go model list
and uses the actual pinned binary's `models` command with isolated XDG paths,
no credentials, and a disabled inference endpoint to validate explicit model
definitions. It does not check account access, quota, or actual generation.
Run it directly without inference:

```sh
adapters/opencode.sh --preflight opencode-go/deepseek-v4.1-flash
```

`adapters/opencode-go-models.json` contains the six cup definitions sourced
from `https://models.dev/api.json` (`opencode-go.models`). The launcher hashes
both this catalog and the helper; update both hashes when changing them.
The same definitions are installed into each guest config, with automatic
model fetching disabled. Missing definitions fail explicitly; no model is
silently substituted. Non-Go routes do not yet have this catalog preflight.

`suites/opencode-cup.toml` pins six Go models verified against the public
`https://opencode.ai/zen/go/v1/models` catalog on 2026-09-12: GLM-5.3,
Kimi K3, MiniMax M3, Qwen3.8 Max, DeepSeek V4.1 Flash, and GPT-5.6 Luna.
All seats use OpenCode, with no Meta or xAI models. This is a separate
season from the Claux fleet; draw it with `season draw`, then run that same
directory with `season run` and the Go credential script shown above.

The cup sets `rules.memory_mib = 2048`, a per-VM floor recorded in new draws.
The runtime passes the manifest allocation to QEMU, overriding the runner's
compiled memory default. Do not infer actual guest memory solely from the
manifest: validate `/proc/meminfo` after changing launch configuration.
It does not change other cups or lower an arena's larger memory allocation.
Unclassified OpenCode errors and exit 137 are harness failures eligible for
the existing bounded replay policy, not player losses. Exit 137 alone is not
proof of OOM; consult the retained VM console. A title requires a durable
final winner. Old draws and results are preserved; use a fresh draw after
these referee/arena changes.

Offline tests run in `make lint`, or separately with
`python3 -m unittest discover -s adapters -p 'test_opencode*.py'`.
These test normalization and the full wrapper using fake SSH/SCP/proxy
processes. Live provider tests are never run by default.

References: [CLI](https://opencode.ai/docs/cli/),
[configuration](https://opencode.ai/docs/config/),
[pinned release](https://github.com/anomalyco/opencode/releases/tag/v1.18.30).
