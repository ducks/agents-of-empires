# Shared model pool

`suites/model-pool.json` is the single model registry for all three cups.
Each model has a stable AoE ID, display name, family, and explicit provider
routes. Each route records the exact upstream model ID, reasoning setting,
eligibility policy, catalog visibility, notes, and optional price metadata.
Go routes also carry the pinned OpenCode client definition; there is no
separate adapter model list.

The initial registry contains the 30 models discussed in the catalog review,
with 27 eligible OpenRouter routes, 16 Go routes, and 25 Vercel routes.
Sol, Astra, and Terra are disabled as premium opt-ins. Vercel's Opus route is
blocked after the observed access failures. Meta and xAI are excluded.
Catalog eligibility is not proof of account access or live harness support.

Cup manifests select a provider rather than copying a fleet:

```toml
[pool]
registry = "model-pool.json"
provider = "vercel"
```

The path resolves relative to the cup manifest. Inline `[[fleet]]` manifests
remain supported, but mixing `fleet` and `pool` is an error. Unknown providers,
duplicate identities/routes, invalid IDs, and missing eligible Go definitions
fail before drawing. `eligible` AND `listed: true` is required for selection.
Other statuses are `disabled` (deliberate opt-in) and `blocked` (known issue).

## Refreshing

```bash
# Public catalogs only; no keys, VMs, inference, or writes.
python3 scripts/refresh-model-pool.py

# Apply metadata and discoveries, never auto-enable them.
python3 scripts/refresh-model-pool.py --write
git diff -- suites/model-pool.json adapters/opencode.sh
nix-shell --run 'make lint'
```

The refresh fetches OpenRouter, Vercel, Go, and models.dev. Empty/invalid
responses abort without writing. Newly discovered tool-capable models/routes
start disabled. Removed IDs remain in the registry with `listed: false`;
existing status, reasoning choices, and model IDs are never overwritten by
catalog data. Unknown Go identities, Meta/xAI, floating aliases, and colon
route variants are not newly imported. Cross-provider IDs with differing
suffixes require manual mapping; similar branding is not proof of equivalence.

`--check` is a read-only automation/CI mode: exit 1 if an update is available,
0 if unchanged. All requests have timeouts. No scheduler is installed.
Prices are provider metadata snapshots, including tiers/discounts where given,
not guaranteed billing or a spending cap. Updates do not change price policy.

To admit a model, review its exact route, cost, reasoning, and account access,
then set its route `status` to `eligible`. After any manual registry edit,
run `--write` to refresh metadata and update the checksum, then review and
test the diff. The two files must travel together: a mismatch fails closed.
The OpenCode launcher hashes the entire registry as part of adapter provenance.

## Drawing and history

Keep the familiar six-player tournament with an explicit entrant count:

```bash
cargo run --release --bin agents-of-empires -- season draw suites/vercel-cup.toml \
  --entrants 6 --week YOUR-NEW-WEEK --output seasons/vercel-cup/YOUR-NEW-WEEK
```

Use the corresponding manifest for other cups. Omitting `--entrants` still
enters the whole resolved pool, which is now larger. Resolution and sampling
happen once during `season draw`; sampled draws snapshot the eligible fleet
and selected entrants. `season run` reads those frozen entries, not today's
registry. Existing draws, results, and report pages are not rewritten.

A refresh changes adapter provenance if Go definitions change. Do not refresh
mid-tournament: finish a started run before updating its harness files. A
catalog refresh cannot restore upstream access to a model that was removed.
