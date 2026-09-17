# term-llm harness adapter

`adapters/term-llm.sh` runs term-llm **0.9.48**, upstream commit
`5d808f7829c05532bd95c5fb0b06f9854b711f9b`, inside the guest. The Linux amd64
release archive is SHA-256 pinned; the launcher also hashes its Python helper.
No floating releases or unchecked local binary overrides are used.

## Routing and isolation

- `openrouter/vendor/model` uses `OPENROUTER_API_KEY` and OpenRouter.
- `vercel/vendor/model` uses `AI_GATEWAY_API_KEY` and Vercel AI Gateway.

Only the prefix is removed. A custom `openai_compatible` provider points to the
loopback reverse-SSH tunnel. The existing host proxy holds the real key; the
guest sees only a placeholder. The built-in OpenRouter provider is intentionally
not used because its endpoint is fixed. `AOE_TERM_LLM_PROXY` may override the
host proxy script location, but not the upstream route.

Each seat gets fresh XDG config/data/cache paths. Sessions, skills, web search,
Guardian approval calls, and subagents are not enabled. The tool set is
read/write/edit files, shell, grep, and glob. Explicit `--approval yolo` is
confined to the disposable guest; it is not a host permission grant. The VM
network and external referee remain the security and scoring boundaries.

The exact model is configured with an internal alias, avoiding ambiguity with
effort suffixes in upstream IDs. `default` omits the effort; other supported
settings are sent explicitly. Provider acceptance still requires a smoke test.
The loop allows 200 turns, with AoE enforcing the match duration externally.

JSONL produces token checkpoints and normalized tool events. Final cumulative
stats replace partial usage sums rather than being added twice. Dollar cost is
unknown, not zero. Tool completion events contain summaries rather than full
stdout; the replay must not imply otherwise. A `done` event alone is not success:
errors, missing terminal evidence, exit codes, and referee reboot markers are
classified separately. AoE alone judges whether a deployment is durable.

Normalized input includes fresh, cache-read, and cache-write tokens. Earlier
adapter cohorts omitted cache categories; historical event logs retain their
original counts. See `harness-exhibition-review.md` for the first run's audit.

## Free checks

Run from `nix-shell`:

```bash
adapters/term-llm.sh --preflight
python3 -m unittest discover -s adapters -p 'test_term_llm*.py'
# Optional: exercise the actual pinned binary against a localhost fake API.
AOE_TEST_TERM_LLM_BINARY="$HOME/.cache/agents-of-empires/term-llm/0.9.48/term-llm" \
  python3 -m unittest discover -s adapters -p 'test_term_llm*.py'
```

Preflight downloads/verifies the binary and checks CLI startup. It does not
validate provider account access, boot a guest, or spend inference. The dynamic
Linux binary needs the nix-ld support already enabled in First Build guests.

## Three-harness exhibition (paid; run explicitly)

`arenas/first-build/agents-harness-exhibition.toml` puts Claux, OpenCode, and
term-llm on the same OpenRouter `openai/gpt-5.6-luna` route with `high` reasoning
and equal VM resources. This is a private exhibition, not a published cup or
model ranking. Harness prompts, tools, compaction, and agent-loop behavior differ.

From Nushell, first enter `nix-shell --run bash`, then:

```bash
: "${OPENROUTER_API_KEY:?Export OPENROUTER_API_KEY first}"
credentials="$(scripts/prepare-infra-core-credentials.sh openrouter)"
cargo run --release --bin agents-of-empires -- run arenas/first-build/agents-harness-exhibition.toml \
  --adapter claux=adapters/claux.sh \
  --adapter opencode=adapters/opencode.sh \
  --adapter term-llm=adapters/term-llm.sh \
  --credential "builder-one=$credentials/builder-one.env" \
  --credential "builder-two=$credentials/builder-two.env" \
  --credential "builder-three=$credentials/builder-three.env" \
  --output "matches/harness-exhibition-$(date -u +%Y%m%d-%H%M%S)"
```

Do not compare raw adapter model strings without removing routing metadata.
Inspect all attempts before adding this harness to a recurring cup. Publishing
and paid runs require separate authorization.
