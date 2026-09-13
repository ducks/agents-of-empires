# Claux through Vercel AI Gateway

AoE's existing Claux adapter supports two routes with the same pinned Claux
release, tools, failure classification, and usage normalization:

| Recorded AoE model | Route | Host credential |
| --- | --- | --- |
| `vendor/model` | OpenRouter (existing default) | `OPENROUTER_API_KEY` |
| `vercel/vendor/model` | Vercel AI Gateway | `AI_GATEWAY_API_KEY` |

The `vercel/` prefix is AoE routing metadata. The adapter removes it before
calling Claux, so Gateway receives `vendor/model`. Use Gateway's exact model
identifier, not an assumed OpenRouter alias. The draw and match manifest keep
the full prefixed identifier, making the route visible in retained evidence.

The pinned Claux `20260908.0.0` already supports `config init --provider vercel`.
No binary upgrade or new agent harness is required.

## Preparing a run

The bundled `suites/vercel-cup.toml` defines a 12-model eligible pool: DeepSeek V4.1
Flash, GPT 5.6 Luna, GLM 5.3, Kimi K3, Qwen 3.8 Max, MiniMax M3, Claude
Sonnet 5, Claude Opus 5, Gemini 3.8 Flash, Devstral 2, Mercury 2.5, and
Nemotron 3.5 Lightning. No Meta or xAI/SpaceXAI models are included.

Pass `--entrants 6` to `season draw` for the usual six-player, two-round cup.
The count is configurable, not a fixed limit. Omit it to enter the whole pool.
Selection uses the published `draw_seed/entrants` shuffle after sorting pool
IDs. Sampled draws freeze both the eligible pool and selected fleet in
`draw.json`; subsequent runs use that snapshot, not the current manifest.
Counts below the arena heat size or above the pool size are rejected; other
sizes use the existing bye/wildcard bracket rules.

This is a broad Gateway cup, not a budget-priced pool. In particular, Opus
can cost materially more than the Flash models. Promotion prices are not
hardcoded into the manifest. The fleet uses 768 MiB per guest, with three
concurrent seats, not twelve concurrent VMs.

`reasoning_effort = "default"` tells this Claux adapter to omit the effort
request. It does not mean reasoning is disabled: the provider/model chooses
its default. The fleet uses this when Gateway lists no effort selector;
Qwen uses `xhigh` because its listed values do not include `high`. Other
entrants request `high`. These settings do not equalize reasoning budgets.

In a new cup manifest, use a distinct season ID such as `vercel-cup`, keep
`adapter = "claux"`, and prefix each verified Gateway model ID with `vercel/`.
Do not edit an existing committed draw to change providers. Model availability
and account access still need checking before drawing the real fleet; this
change does not add a Gateway catalog or account-quota preflight.

Use the already-exported `AI_GATEWAY_API_KEY`. In Bash inside `nix-shell`
(from Nushell, enter `nix-shell --run bash` first):

```bash
week="$(date -u +%G-W%V-%Y%m%d-%H%M%S)"
tournament="seasons/vercel-cup/$week"
cargo run --release --bin agents-of-empires -- season draw suites/vercel-cup.toml \
  --entrants 6 --week "$week" --output "$tournament"

credentials="$(scripts/prepare-infra-core-credentials.sh vercel)"
credential_args=()
for file in "$credentials"/*.env; do
  credential_args+=(--credential "$(basename "$file" .env)=$file")
done
# Once a new Vercel draw has been created, running it spends inference:
cargo run --release --bin agents-of-empires -- season run "$tournament" \
  --adapter claux=adapters/claux.sh "${credential_args[@]}"
```

The credential helper creates twelve territory files in a private temporary
directory, with mode 0600. Its default without an argument remains OpenRouter.

## Boundary and accounting

The real key stays in the host proxy environment. The guest receives only
`AI_GATEWAY_API_KEY=arena-proxy-placeholder` and a loopback endpoint reached
through the controller's reverse SSH tunnel. Gateway requests use `/v1`;
OpenRouter requests continue using `/api/v1`. Upstream selection is explicit,
so inherited proxy endpoint/key variables cannot silently change the route.

The proxy remains the shared Replaybook host proxy. `AOE_CLAUX_PROXY` can
override its script path; `AOE_OPENROUTER_PROXY` remains a legacy fallback.
Neither override changes the selected provider endpoint.

Provider-reported costs are retained when Claux supplies them. Missing costs
remain null in normalized usage; zero shown by an aggregate is not proof of
free inference. No Gateway price estimate or subscription assumption is added.

Offline adapter tests cover routing, placeholder-only guest commands, usage,
authentication failures, malformed model IDs, omitted reasoning effort, and
missing Gateway keys. The GLM 5.3 Flash smoke run
`matches/vercel-smoke-20260913-161816` completed a reboot-verified deployment
in 28.672 seconds with no infrastructure failures and $0.009741 in final
recorded usage across three seats. This verified the route, not the entire
fleet: GLM 5.3 in the cup is a different model, and the other entrants still
need live validation before treating the cup as tested.
