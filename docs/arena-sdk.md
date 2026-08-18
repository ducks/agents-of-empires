# Arena SDK

An arena is a portable directory that defines the guest machines, instructions, scoring milestones, and external verification for one kind of infrastructure contest. It does not contain model-provider logic. Any registered agent adapter can compete against the same arena.

## Create an arena

```bash
agents-of-empires arena init cache-race
agents-of-empires arena validate arenas/cache-race
```

`arena init` creates a complete, runnable build race with three equivalent NixOS territories, controller-owned verifier scripts, an oracle adapter, a credential helper, and a smoke test. It refuses to overwrite a non-empty directory.

The package layout is:

```text
arena.toml                 versioned arena and scoring contract
flake.nix                  disposable NixOS territory definitions
CONTRACT.md                service outcome promised to every agent
instructions/<seat>.md     private starting instructions per territory
verify/*.sh                controller-owned milestone verifiers
adapters/oracle.sh         deterministic reference implementation
scripts/                   local credential/setup helpers
tests/smoke.sh             package validation smoke test
README.md                  arena-specific usage and design notes
```

Relative paths are resolved from the arena directory. This makes a package runnable from another repository or an absolute checkout path. Verifiers and Nix flake references may not escape the package. Public internet access inside territories remains forbidden by the manifest schema.

## Fog of war

Discovery-first arenas can replace the contract and seat-specific instructions with one deliberately narrow player brief:

```toml
[fog_of_war]
player_brief = "brief.md"
hide_topology_until_observed = true

[fog_of_war.guest_leak_audit]
scan_paths = ["/etc", "/opt", "/root", "/var/lib"]
forbidden_strings = ["private verifier name", "oracle-only clue"]
```

The brief contains the objective, observable symptoms or public service contract, deadline, and operational constraints. It does not expose the implementation, root cause, verifier scripts, evidence, future events, or other territories. The controller keeps those parts of the arena package outside the guest and scans the configured guest paths before launching agents. A match aborts if a forbidden referee or oracle clue crossed that boundary.

Fog mode also records the player brief digest in `match.json`, so changing what players were told creates a new compatibility key. In static replays, topology nodes and links remain unknown until their associated milestones are first observed. This visual concealment is presentation; guest isolation and the pre-launch audit are the security boundary.

## Validation contract

```bash
agents-of-empires arena validate path/to/arena
agents-of-empires arena validate path/to/arena/arena.toml --json
```

Validation checks the schema and milestone DAG, one agent per territory, isolated networking, package-relative Nix references, executable verifier scripts, territory instructions, and the durable build contract. Missing documentation, oracle, smoke test, or lock file is reported separately as a warning.

Before publishing an arena, run:

```bash
nix flake lock path/to/arena
nix flake check path/to/arena
AOE_BIN="$(pwd)/target/release/agents-of-empires" path/to/arena/tests/smoke.sh
```

The oracle should then complete the arena before paid models are used. A harness cannot redefine success: adapters invoke agents and normalize their output, while arena verifiers remain the sole authority on milestones and durable completion.

## Optional service map

Build arenas can describe their topology for match replays without changing the
verifier or scoring contract. Nodes may point at a milestone, and the static
report projects that node through pending, verifying, healthy, failed, and
durable states from the immutable referee event stream.

```toml
[visualization]

[[visualization.nodes]]
id = "api"
display_name = "Job API"
kind = "service"
milestone = "service-up"
x = 30
y = 40

[[visualization.nodes]]
id = "queue"
display_name = "Durable Queue"
kind = "queue"
milestone = "recover-accepted"
x = 70
y = 40

[[visualization.links]]
from = "api"
to = "queue"
kind = "queue"
label = "enqueue"
```

Coordinates are percentages from `0` to `100`. Node kinds are `client`,
`proxy`, `service`, `worker`, `queue`, `database`, `storage`, and `host`. Link
kinds are `traffic`, `queue`, `replication`, `storage`, and `lifecycle`.
Milestone references and link endpoints are validated when the arena loads.

Each match records the exact manifest as `arena.json`. Report generation uses
that snapshot, so external packages receive the same visualization support and
historical matches without a snapshot continue to render without a map.

## Optional reporting category

An arena may opt into the benchmark capability map with a presentation-only
category in its existing `[arena]` table:

```toml
[arena]
id = "durable-job-queue"
display_name = "Durable Job Queue"
category = "stateful-recovery"
```

Changing this category does not change the evaluation compatibility key.
Historical benchmark JSON without the field remains valid; known built-in
arenas receive a reporting fallback and other arenas appear as Uncategorized.

## Schema versioning

Every `arena.toml` begins with `schema_version = 1`. Unknown fields and unsupported schema versions fail validation. Future incompatible arena contracts will receive a new schema version instead of silently changing historical matches.
