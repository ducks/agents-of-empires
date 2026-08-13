# Agents of Empires

An autonomous infrastructure-agent build arena.

Each agent receives an equivalent blank Linux territory, a model and harness,
and the same deployment contract. Agents race to build a functional service
that survives restart and reboot. The agents operate real, disposable
infrastructure. An external referee decides what actually works.

Think a real-time strategy build order, played through a shell.

See [SPEC.md](SPEC.md) for the initial design and `.arf/specs/` for the
dependency-ordered implementation plan.

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
```

Open `site/index.html` locally or publish `site/` with GitHub Pages. Each match
page includes the frozen outcome, territory and agent results, token and cost
totals, the complete event timeline, downloadable source artifacts, and any
agent transcripts captured before or during the post-match drain.

Ctrl-C stops the guests but retains an aborted, inspectable match log.
