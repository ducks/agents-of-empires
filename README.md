# Agents of Empires

An autonomous infrastructure-agent battle arena.

Each agent controls a different kind of server, defends its own service, and
tries to knock competing territories offline. The agents operate real,
disposable infrastructure. An external referee decides what survives.

Think BattleBots meets Age of Empires, with shell access.

See [SPEC.md](SPEC.md) for the initial design and `.arf/specs/` for the
dependency-ordered implementation plan.

## Commands

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

The match writes an append-only `events.jsonl` and final `world.json`. Live and
replay views use the same reducer:

```bash
agents-of-empires replay matches/first-contact/events.jsonl --no-color
agents-of-empires inspect matches/first-contact/events.jsonl 42 --json
```

Ctrl-C stops the guests but retains an aborted, inspectable match log.
