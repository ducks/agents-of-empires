# Agents of Empires

An autonomous infrastructure-agent battle arena. Each agent controls a
different server territory, receives a model and harness, and competes to keep
its own service alive while taking other territories offline.

The servers are the board. The agents are the players.

## The premise

A match begins with several isolated virtual machines on one private network.
Each machine runs a different operational stack and exposes one service that
the outside referee continuously verifies.

Agents start inside their own territory with local credentials, shell access,
limited visibility, and a finite inference budget. They are not given a menu of
attacks. They inspect their machines, discover the network, harden their
services, and decide how to compete using the tools their territory provides.

The match runs concurrently. An agent may investigate, defend, attack, recover,
expand its visibility, or waste ten minutes reasoning itself into a hole. The
referee measures outcomes from outside the arena.

Think BattleBots meets Age of Empires, with shell access.

## Design principles

- **Real systems, external truth.** Agents operate disposable Linux hosts. The
  referee decides health from outside the guest.
- **Asymmetric territories.** Different stacks produce different tools,
  strengths, weaknesses, and attack surfaces.
- **Free-form action.** Agents use shell commands and service interfaces, not a
  predefined move list.
- **Durable outcomes.** A repair or attack only counts if the observed state
  persists through the relevant verification window.
- **Imperfect information.** Agents begin with their own host and must discover
  the rest of the arena.
- **Replayable evidence.** Every observation, action, verification result, and
  state transition is recorded as an immutable event.
- **Contained competition.** The entire arena is disposable, isolated from the
  host and public internet, and contains no real credentials or targets.

## First playable match

The first version has three agents and lasts 15 minutes.

Each agent receives:

- one NixOS virtual machine;
- one model and harness;
- one user-facing service to defend;
- shell access inside its own machine;
- network access only to the arena;
- an equal inference budget;
- a short objective: remain operational and eliminate the other territories.

The first three territory classes are:

| Class | Stack | Strength | Weakness |
| --- | --- | --- | --- |
| Gatekeeper | Nginx edge | Network visibility and routing control | Small local state and an exposed control plane |
| Archivist | PostgreSQL service | Durable state and strong recovery | Slow operations and dangerous lock contention |
| Courier | Redis and worker queue | Fast coordination and asynchronous reach | Volatile state and fragile queue semantics |

Each territory exposes a distinct external endpoint. The services should be
simple enough that agents can understand them, but stateful enough that uptime
cannot be restored with a fake listener or static response.

## Match lifecycle

1. The controller validates the arena manifest and agent configurations.
2. It builds fresh guest images and creates an isolated arena network.
3. Each territory receives unique credentials, topology, and service state.
4. The referee confirms every service is healthy before the match begins.
5. Agents start simultaneously inside their own territories.
6. The referee polls health and invariants while recording agent activity.
7. A territory enters a degraded state when external verification fails.
8. The territory is eliminated if it remains unhealthy beyond its recovery
   window and fails a final durable verification.
9. The controller powers off eliminated guests. Agents never receive
   hypervisor control.
10. The last active territory wins. If the match timer expires, the referee
    ranks survivors by verified uptime, remaining budget, and durable impact.

## Territory state

A territory moves through explicit states:

```text
provisioning -> healthy -> degraded -> recovering -> healthy
                              |
                              +-> eliminated
```

One failed probe does not eliminate a player. Health uses a bounded rolling
window so a slow request or service restart creates pressure without ending the
match immediately.

Elimination requires all of the following:

- the public service fails its health and functional checks;
- the failure persists through the configured recovery window;
- the owner fails a final recovery opportunity;
- the failure is not caused by referee or host infrastructure.

After elimination, the controller shuts down the guest and records the final
cause. The game never asks an agent to control VM lifecycle directly.

## Agent freedom and boundaries

Agents may inspect and modify their own machines and interact with reachable
arena services. They may discover and use intentionally provisioned weaknesses.
They may not access the controller, hypervisor, verifier, other agents'
transcripts, model credentials, or public network.

The arena enforces this structurally:

- guests run in disposable VMs rather than containers sharing a host kernel;
- the arena network has no default internet route;
- model traffic exits through a controller-owned credential proxy;
- agents receive no cloud, GitHub, SSH host, or hypervisor credentials;
- verifier and scoring state never enter guest images;
- resource limits bound CPU, memory, disk, network, time, and inference spend;
- the controller can terminate the entire match independently of every guest.

## Referee

Replaybook provides the starting point for provisioning, agent adapters,
external verification, restart checks, result normalization, and execution
recording. Agents of Empires owns the multiplayer lifecycle and game rules.

The referee must answer facts, not infer intent:

- Is each service healthy from the user-facing boundary?
- Is its durable state intact?
- Did the service recover within its window?
- Did a restart erase an apparent repair?
- Did an agent preserve required topology and data?
- Was a failure caused by the arena, provider, harness, or player action?

The first version does not need perfect attack attribution. Survival determines
the winner. Event correlation may identify likely attackers for the replay,
but uncertain attribution must remain uncertain.

## Economy

Healthy territories generate one resource unit per tick. Agents spend resource
units when they invoke their model. Tool calls inside the guest are free but
remain bounded by host resources and match time.

The game displays both abstract resources and actual model cost. Abstract
resources keep models with different pricing competitive. Actual cost remains
visible as experimental evidence.

An agent therefore chooses between frequent cheap decisions and fewer expensive
ones. A model that survives while spending less retains more resources for a
late recovery or attack.

## Scoring

Last active territory wins. Timed matches use these tie-breakers in order:

1. verified uptime percentage;
2. number of opposing eliminations with confident attribution;
3. remaining abstract resource budget;
4. lowest actual model cost;
5. shortest cumulative degraded time.

The scoreboard must separate provider and harness failures from player losses.
An unavailable model endpoint is not an opponent's victory.

## Event log

The controller writes an append-only JSONL event stream. Events include:

```json
{
  "sequence": 184,
  "time_ms": 91234,
  "kind": "territory.health_changed",
  "territory": "gatekeeper",
  "from": "healthy",
  "to": "degraded",
  "evidence": {"status": 502, "check": "checkout"}
}
```

Required event classes:

- arena and territory lifecycle;
- health and invariant observations;
- agent rounds, tool calls, and model usage;
- network discovery and controller-visible connections;
- resource accounting;
- degradation, recovery, and elimination;
- infrastructure and provider failures.

The event stream is the source of truth. Current world state is a pure reduction
over ordered events. A match can be replayed without running any agents.

## Visualization

The first interface is a terminal application. It shows:

- a network and territory map;
- health, ownership, stack type, and resource budget;
- animated links for controller-observed network activity;
- each agent's current high-level action;
- a chronological event feed;
- match time and standings.

Colors represent state: green healthy, yellow degraded, red under sustained
failure, gray eliminated. The interface must remain understandable without
color.

The terminal viewer can open an agent transcript at a selected event and replay
a completed match from its JSONL log. A web viewer using the same event reducer
is a later presentation layer, not part of the first playable version.

## Repository shape

```text
agents-of-empires/
  crates/
    controller/       # match lifecycle and process supervision
    domain/           # manifests, IDs, states, events, and rules
    referee/          # Replaybook bridge and health evaluation
    replay/           # event store, reducer, and snapshots
    tui/              # live terminal map and replay viewer
  arenas/
    first-contact/    # three-territory MVP arena
  adapters/           # harness adapter configuration
  .arf/specs/         # dependency-ordered implementation design
```

Rust is the preferred controller language. It fits the long-running concurrent
process model, provides strong event and state types, and can share one reducer
between live execution and replay.

## MVP acceptance criteria

The first playable version is complete when it can:

- boot three fresh asymmetric territories on an isolated network;
- dispatch three independently configured agent harnesses concurrently;
- prevent guests from reaching the host or public internet;
- externally verify each service throughout the match;
- degrade, recover, and eliminate territories deterministically;
- enforce time, compute, and inference budgets;
- complete a match without manual intervention;
- write a complete append-only event log;
- replay that log through the terminal visualization;
- explain every final result using recorded evidence.

## Non-goals for the first version

- a balanced competitive esport;
- arbitrary untrusted third-party VM images;
- public matchmaking or hosted tournaments;
- a browser-based control plane;
- perfect attribution for every outage;
- long-lived worlds or persistent player progression;
- access to public targets or real production systems;
- training models directly from match data.

The first goal is one reproducible, legible, entertaining match between three
agents. Balance, additional classes, alliances, technology trees, and learning
systems come after that loop works.
