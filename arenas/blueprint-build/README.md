# Blueprint Build

Three agents receive the same architecture diagram, a nearly blank NixOS host,
and standardized lifecycle slots. The diagram specifies behavior and component
boundaries without choosing a language, database, queue, or internal protocol.
The first implementation to accept work asynchronously, recover exact opaque
payloads, survive independent service restarts, and survive host reboot wins.

Validate the package and prove the contract with the deterministic oracle:

```bash
cargo run --release --bin agents-of-empires -- arena validate arenas/blueprint-build
credentials="$(scripts/prepare-blueprint-credentials.sh)"
cargo run --release --bin agents-of-empires -- run \
  arenas/blueprint-build/arena.toml \
  --adapter oracle-blueprint=adapters/oracle-blueprint.sh \
  --credential blueprint-one="$credentials/blueprint-one.env" \
  --credential blueprint-two="$credentials/blueprint-two.env" \
  --credential blueprint-three="$credentials/blueprint-three.env" \
  --output "matches/blueprint-build-oracle-$(date -u +%Y%m%d-%H%M%S)"
```

For a real race, substitute `agents-real.toml` and register the bundled `claux`
adapter. Claux receives `blueprint.png` through its repeatable image argument.
