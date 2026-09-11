# Repository instructions

## Workflow

- Work on a feature branch. Do not commit directly to `main`.
- Use merge commits (`git merge --no-ff`) when a completed feature branch is
  merged into `main`, and only merge after `make lint` passes.
- Preserve retained match, series, benchmark, and season artifacts under
  `matches/`, `series/`, `benchmarks/`, and `seasons/`. They are evidence.
- Do not push, publish the site, create a release or tag, or spend inference
  unless the user explicitly asks.
- Enter `nix-shell` before building or running an arena. It pins the Rust
  toolchain and the Nix, QEMU, and SSH tools that `agents-of-empires doctor`
  checks.

## Project map

- `crates/domain/` owns manifests, milestones, competitor and agent states,
  events, and validation. Every scoring concept is defined here.
- `crates/referee/` evaluates milestones from outside the guest and records
  the frozen result.
- `crates/runtime/` builds and boots the disposable NixOS territories and the
  sealed management network.
- `crates/agent/` executes harness adapters and normalizes their results;
  `crates/agent/src/protocol.rs` is the adapter contract.
- `crates/replay/` holds the append-only event log and the reducer that both
  live and replay views use.
- `crates/controller/` is the CLI: `runner.rs` runs one match, `series.rs`
  rotates seats, `benchmark.rs` runs a suite, `season.rs` draws and runs a
  weekly bracket, `report.rs` renders the static site, `trajectory.rs`
  exports ATIF.
- `crates/tui/` is the live milestone race view.
- `arenas/` are self-contained arena packages; `adapters/` are harness
  adapters; `suites/` hold benchmark and season manifests; `scripts/`
  prepare per-territory credential files.
- `docs/arena-sdk.md` is the arena package contract.

## Running things

Validate and preflight without spending anything:

```sh
cargo run --release --bin agents-of-empires -- doctor
cargo run --release --bin agents-of-empires -- validate arenas/first-build/arena.toml
make demo        # free oracle race on real guests, generates its report
```

Every real run needs a credential file per territory and an adapter mapping.
Credential files are produced by `scripts/prepare-*-credentials.sh` and hold
the SSH password plus the provider key that the controller-owned proxy uses.
The real key never enters a guest.

One match, a seat-rotated series, and a benchmark suite:

```sh
export OPENROUTER_API_KEY=...
credentials="$(scripts/prepare-first-build-credentials.sh)"
cargo run --release --bin agents-of-empires -- run arenas/first-build/agents-real.toml \
  --adapter claux=adapters/claux.sh \
  --credential builder-one="$credentials/builder-one.env" \
  --credential builder-two="$credentials/builder-two.env" \
  --credential builder-three="$credentials/builder-three.env" \
  --output "matches/first-build-$(date -u +%Y%m%d-%H%M%S)"

cargo run --release --bin agents-of-empires -- series arenas/first-build/agents-real.toml \
  --adapter claux=adapters/claux.sh --credential ... --output series/first-build-...

credentials="$(scripts/prepare-infra-core-credentials.sh)"
credential_args=()
for file in "$credentials"/*.env; do
  credential_args+=(--credential "$(basename "$file" .env)=$file")
done
cargo run --release --bin agents-of-empires -- benchmark suites/infra-core.toml \
  --adapter claux=adapters/claux.sh "${credential_args[@]}" \
  --output "benchmarks/infra-core-$(date -u +%Y%m%d-%H%M%S)"
```

A weekly season is a two-step workflow. Commit the draw first; it needs no
credentials and no network:

```sh
cargo run --release --bin agents-of-empires -- season draw suites/weekly-season.toml \
  --week 2026-W37 --output seasons/infra-weekly/2026-W37
```

The draw writes `draw.json` (the bracket, arena per round, and every seat,
all derived from the published seed `season-id/week[/salt]`) and
`seed.secret` (mode 0600). Re-running the same draw is refused; use a new
week directory. Then run the week with credentials for every territory of
every arena in the pool, which `prepare-infra-core-credentials.sh` provides
for the bundled pool:

```sh
cargo run --release --bin agents-of-empires -- season run seasons/infra-weekly/2026-W37 \
  --adapter claux=adapters/claux.sh "${credential_args[@]}"
```

`season run` checkpoints `week.json` after each heat and resumes a
compatible checkpoint. Re-run the same command after an interruption. A
season can also be inspected with `cat seasons/infra-weekly/2026-W37/week.json`.

Render everything into the static site, then open `site/index.html`:

```sh
cargo run --release --bin agents-of-empires -- report matches \
  --series series --benchmark benchmarks --season seasons/infra-weekly --output site
```

Use `--base-port` when the default block starting at `26000` is busy;
series, benchmarks, and seasons allocate two ports per territory per
concurrent match from that base.

## Tournament workflow and publishing

- AoE is a spectator tournament, not a model-ranking product. The homepage
  leads with the next draw, latest completed tournament, and past brackets.
  Series, benchmarks, and standalone matches remain accessible in the archive.
- CLI terminology remains `season draw` / `season run`: a season is the
  recurring fleet and arena pool; each week directory is one tournament.
- A morning draw and a later run are separate operations. Draw-only reports
  are supported: they show the opening matchups without scores or a champion.
  Drawing and rendering spend no inference; running the bracket does.
- Use a unique week label and output directory. Preserve `draw.json` and its
  private `seed.secret`; do not redraw, edit a committed draw, or reuse a
  completed tournament directory to get a different result. Resume an
  interrupted tournament with `season run` against that same directory.
- Reuse the user's exported `OPENROUTER_API_KEY`; do not prompt for it again
  or print it. The credential-array examples here are Bash. From Nushell,
  enter `nix-shell --run bash` before using them.
- To publish a draw, render both the selected completed tournaments and the
  new draw with repeated `--season` arguments. Do not pass the entire local
  season root unless every test tournament under it is intended to be public.

```sh
# Draw now (no VMs), then run this exact bracket later, only when requested:
cargo run --release --bin agents-of-empires -- season draw suites/weekly-season.toml \
  --week 2026-W37-20260911 --output seasons/infra-weekly/2026-W37-20260911
# After preparing credential_args as above:
cargo run --release --bin agents-of-empires -- season run seasons/infra-weekly/2026-W37-20260911 \
  --adapter claux=adapters/claux.sh "${credential_args[@]}"

# Render only the public completed tournament plus the upcoming draw:
cargo run --release --bin agents-of-empires -- report matches \
  --series series --benchmark benchmarks \
  --season seasons/infra-weekly/2026-W37-fixed-20260910-153524 \
  --season seasons/infra-weekly/2026-W37-20260911 --output site
```

- GitHub Pages serves the root of `gh-pages`. Publish generated report files
  only, never raw run directories, credentials, VM disks, or `seed.secret`.
  Review exports for secrets and broken links before pushing. Preserve
  existing public archive pages. Confirm the Pages build and public URL.
- Publishing requires explicit user authorization. A draw publication is not
  authorization to run the tournament or schedule paid inference. After the
  run, regenerate and republish the same bracket URL when requested.

## Behavioral contracts

- The controller and referee are the scoring boundary. Adapters may change
  how a harness runs; they must not change what counts as success.
- The real provider key never enters a guest. Adapters receive it only
  through the controller-owned credential proxy and a controller-owned
  credential file. Never print or commit a credential file.
- Verifier secrets, oracle material, and `seed.secret` stay controller-side.
  Only the variation seed's commitment is public until the week completes;
  `week.json` then reveals the seed so anyone can check it.
- A match, series, benchmark, or week that a rival wins is a player outcome.
  Agents still building when the drain expires are recorded as outraced and
  `incomplete`, ranked by verified milestones, never as a controller failure.
  Provider and harness failures are `unavailable` and never count as a loss.
- Heats and matches stop at the first durable deployment
  (`stop_on_first_durable`). Losing the race is the result; do not add
  post-win evaluation to arenas.
- Draws are reproducible from their published seed. Never default a run to a
  floating "latest" harness release; `adapters/claux.sh` pins a release and
  its SHA-256 together, and a bump is a deliberate cohort boundary.
- Keep results from different harnesses, arena versions, fleets, and
  compatibility keys visibly separate. Weeks draw arenas independently and
  are not comparable to each other as benchmarks; the benchmark suite is the
  comparable measurement.
- The event log is append-only and the reducer is the single source of truth
  for live, replay, and report views. Add an event variant rather than
  reinterpreting an existing one, so old logs reduce unchanged.

## Testing and validation

- Add focused tests with each behavioral change. The controller has unit
  tests per module and integration tests under `crates/controller/tests/`;
  adapter behavior is covered by `adapters/test-claux.sh` with fake `ssh`,
  `scp`, and `curl` on `PATH`.
- Run the narrowest relevant test while iterating, then before handoff:

  ```sh
  make lint
  ```

  This checks formatting, runs Clippy with pedantic warnings across every
  target, runs the workspace tests, validates the example arena package, and checks
  the demo launcher.
- Changes to scoring, the reducer, or the adapter contract should also run
  `make demo`, which races three oracle agents on real guests without model
  spend and verifies the winner through a host reboot.
- `season draw` is safe to run against the real manifests in tests; it
  touches no guests. `season run`, `run`, `series`, and `benchmark` build and
  boot VMs and, with a real adapter, spend inference. Only run them when the
  user asks.

## Configuration and adapters

- A new harness is an adapter script implementing the environment and result
  contract in `crates/agent/src/protocol.rs`; see `adapters/claux.sh` and
  `adapters/test-claux.sh`. Map the harness's exit codes and any classified
  failure it reports onto `completed`, `failed`, `unavailable`,
  `interrupted`, and `harness_error`; do not let a present result file imply
  success.
- A new arena is a package under `arenas/` scaffolded by `arena init` and
  validated by `arena validate`; see `docs/arena-sdk.md`. Verifiers may read
  `AOE_SCENARIO_SEED` when present and must keep generating their own random
  values when it is absent.
- Season manifests live in `suites/` next to benchmark suites. Every arena
  in a season pool must declare the same number of territories.

## Releases

- Versions are date-based. `make release` runs the preflight, bumps the
  workspace version on a release branch, merges it with a merge commit, tags,
  pushes, and triggers the binary build on GitHub. It is externally mutating
  and must only run after the user explicitly requests a release.
