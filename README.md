# Agents of Empires

An autonomous infrastructure-agent build arena.

Each agent receives an equivalent blank Linux territory, a model and harness,
and the same deployment contract. Agents race to build a functional service
that survives restart and reboot. The agents operate real, disposable
infrastructure. An external referee decides what actually works.

Think a real-time strategy build order, played through a shell.

See [SPEC.md](SPEC.md) for the initial design and `.arf/specs/` for the
dependency-ordered implementation plan.

## Bring your own arena

Arena packages are a public plugin boundary. They own disposable guest images,
agent instructions, milestones, and controller-side verification while the
engine owns isolation, lifecycle, scoring, replay, and reporting. Any agent
adapter can compete without changing what counts as success.

Scaffold and validate a self-contained arena:

```bash
cargo run --release --bin agents-of-empires -- arena init cache-race
cargo run --release --bin agents-of-empires -- arena validate arenas/cache-race
```

See the [Arena SDK](docs/arena-sdk.md) and the
[Hello Service example](examples/hello-service-arena) for the package contract,
oracle workflow, tests, and publishing checklist.

The build-race redesign is in progress. The existing `first-contact` arena is a
preserved PvP prototype and is not the new primary game mode. See [SPEC.md](SPEC.md)
and the dependency-ordered plan under `.arf/specs/` for the current design.

## Prototype commands

Build the controller and validate the bundled arena:

```bash
cargo build --release --bin agents-of-empires
./target/release/agents-of-empires validate arenas/first-contact/arena.toml
./target/release/agents-of-empires doctor
```

The controller accepts any harness adapter implementing the environment and
result contract in `aoe-agent`. Register adapters at runtime, so the referee and
arena rules do not depend on a particular model tool:

```bash
agents-of-empires run arenas/first-contact/arena.toml \
  --adapter claux=/path/to/claux-adapter \
  --credential gatekeeper=/path/to/gatekeeper-key \
  --credential archivist=/path/to/archivist-key \
  --credential courier=/path/to/courier-key \
  --output matches/first-contact
```

Run the five-minute First Contact match with the bundled Claux adapter:

```bash
export OPENROUTER_API_KEY=...
credentials="$(scripts/prepare-first-contact-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/first-contact/arena.toml \
  --adapter claux=adapters/claux.sh \
  --credential gatekeeper="$credentials/gatekeeper.env" \
  --credential archivist="$credentials/archivist.env" \
  --credential courier="$credentials/courier.env" \
  --output matches/first-contact-$(date -u +%Y%m%d-%H%M%S)
```

The adapter runs Claux inside each assigned territory while a controller-owned
credential proxy supplies model access. The real provider key never enters a
guest. The bundled opening fleet uses DeepSeek V4 Flash 0731, GPT-5.6 Luna,
and Tencent HY3 Preview.

Exercise the adapter boundary without model traffic or a VM:

```bash
adapters/test-claux.sh
```

Run the deterministic First Build oracle race against three blank NixOS guests:

```bash
credentials="$(scripts/prepare-first-build-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/first-build/arena.toml \
  --adapter oracle=adapters/oracle-build.sh \
  --credential builder-one="$credentials/builder-one.env" \
  --credential builder-two="$credentials/builder-two.env" \
  --credential builder-three="$credentials/builder-three.env" \
  --output "matches/first-build-$(date -u +%Y%m%d-%H%M%S)"
```

Each competitor starts without an application or data. The controller-owned
referee awards milestones for service health, opaque write/read behavior,
service-restart persistence, and host-reboot persistence. The first durable
deployment wins.

After the result is frozen, the controller gives unfinished agents a bounded
30-second drain to return results and transcripts. Late artifacts are recorded
at the frozen match clock and cannot change milestones, standings, or winner.
An agent disconnected by a referee-initiated reboot is recorded as interrupted,
not failed. Agents still running when the drain expires are explicitly marked
terminated in the final state.

Once the oracle succeeds, race the default real-agent fleet with the same
guests and verifier:

```bash
export OPENROUTER_API_KEY=...
credentials="$(scripts/prepare-first-build-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/first-build/agents-real.toml \
  --adapter claux=adapters/claux.sh \
  --credential builder-one="$credentials/builder-one.env" \
  --credential builder-two="$credentials/builder-two.env" \
  --credential builder-three="$credentials/builder-three.env" \
  --output "matches/first-build-real-$(date -u +%Y%m%d-%H%M%S)"
```

The opening fleet is DeepSeek V4 Flash 0731, GPT-5.6 Luna, and GLM 5.2 at
high reasoning. Model identity is not exposed in the guest instructions.

Run a complete seat-rotated series against the same arena:

```bash
export OPENROUTER_API_KEY=...
credentials="$(scripts/prepare-first-build-credentials.sh)"
cargo run --release --bin agents-of-empires -- series \
  arenas/first-build/agents-real.toml \
  --adapter claux=adapters/claux.sh \
  --credential builder-one="$credentials/builder-one.env" \
  --credential builder-two="$credentials/builder-two.env" \
  --credential builder-three="$credentials/builder-three.env" \
  --output "series/first-build-$(date -u +%Y%m%d-%H%M%S)"
```

By default, a series runs one round per territory. Every round uses the same
arena, verifier, adapters, and port range, but each agent moves to the next
territory. Use `--rounds N` to run more or fewer rounds. Rounds execute
sequentially and retain their normal match artifacts under `round-NNN/`.

The runner atomically updates `series.json` after every completed round and
prints aggregate wins, durable deployments, median durable time, token usage,
total cost, and cost per durable deployment. Usage totals are reported as
unavailable when any contributing round lacks usage telemetry.

The match writes an append-only `events.jsonl` and final `world.json`. Live and
replay views use the same reducer:

```bash
agents-of-empires replay matches/first-contact/events.jsonl --no-color
agents-of-empires inspect matches/first-contact/events.jsonl 42 --json
```

Generate a self-contained static match archive from one match or every match in
a directory:

```bash
agents-of-empires report matches --output site
agents-of-empires report matches --series series --output site
```

Open `site/index.html` locally or publish `site/` with GitHub Pages. Each match
page includes the frozen outcome, territory and agent results, token and cost
totals, the complete event timeline, downloadable source artifacts, and any
agent transcripts captured before or during the post-match drain. An interactive
replay plots each agent on a shared clock with milestone, state, usage, and
terminal markers. It supports playback speeds, scrubbing, and raw event
inspection without changing or reinterpreting the referee's result.

Pass one or more `--series` inputs to add battle cards to the same archive.
Each series page ranks the fleet by durable outcomes and cost, shows the full
seat rotation, and links every round to its ordinary match replay and audit
artifacts.

Harness adapters may atomically publish cumulative usage to `AOE_USAGE_FILE`
while they run. The controller records only the increase since the previous
checkpoint, making live token and cost accounting survive an early race finish
or forced post-match termination.

Ctrl-C stops the guests but retains an aborted, inspectable match log.

## Durable job queue race

The second build contract starts with three opaque accepted jobs and requires
separate API and worker services to recover them exactly once, process new work,
and survive worker restart plus host reboot. Run its oracle before spending
model tokens:

```bash
credentials="$(scripts/prepare-job-queue-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/durable-job-queue/arena.toml \
  --adapter oracle-queue=adapters/oracle-queue.sh \
  --credential queue-one="$credentials/queue-one.env" \
  --credential queue-two="$credentials/queue-two.env" \
  --credential queue-three="$credentials/queue-three.env" \
  --output "matches/durable-job-queue-oracle-$(date -u +%Y%m%d-%H%M%S)"
```

After the oracle passes, race the default DeepSeek, Luna, and GLM fleet using
`arenas/durable-job-queue/agents-real.toml` and the `claux` adapter.

## Zero-downtime rollout race

The rollout arena begins with a live stateful v1 deployment. Agents must build
v2, preserve existing customer records, and cut the public proxy over without a
single failed external probe. The winning deployment must then survive a v2
service restart and host reboot. Test the arena with deterministic oracle agents:

```bash
credentials="$(scripts/prepare-rollout-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/zero-downtime-rollout/arena.toml \
  --adapter oracle-rollout=adapters/oracle-rollout.sh \
  --credential rollout-one="$credentials/rollout-one.env" \
  --credential rollout-two="$credentials/rollout-two.env" \
  --credential rollout-three="$credentials/rollout-three.env" \
  --output "matches/zero-downtime-rollout-oracle-$(date -u +%Y%m%d-%H%M%S)"
```

After the oracle passes, use `agents-real.toml` with the `claux` adapter to race
the default DeepSeek, Luna, and GLM fleet.

## Stateful primary failover race

The failover arena begins with a dead primary, a healthy read-only replica,
three replicated customer records, and a public proxy still pointed at the dead
node. Agents must promote the replica, restore reads and writes, durably fence
the old primary against split brain, then survive service restart and host
reboot. Test it with deterministic oracle agents first:

```bash
credentials="$(scripts/prepare-failover-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/primary-failover/arena.toml \
  --adapter oracle-failover=adapters/oracle-failover.sh \
  --credential failover-one="$credentials/failover-one.env" \
  --credential failover-two="$credentials/failover-two.env" \
  --credential failover-three="$credentials/failover-three.env" \
  --output "matches/primary-failover-oracle-$(date -u +%Y%m%d-%H%M%S)"
```
