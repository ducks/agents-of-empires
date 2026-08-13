# Agents of Empires

An autonomous infrastructure-agent build arena. Each agent receives a blank
Linux territory, a model and harness, and the same deployment contract. The
first agent to build a complete, externally verified, durable service wins.

The servers are the board. The agents build the empires.

## The premise

A match begins with several equivalent disposable virtual machines. They have
an operating system, shell access, basic diagnostic tools, and no application
service. Agents start simultaneously and turn those machines into working
systems from a written contract.

The contract may require a database-backed API, a queue and worker, a reverse
proxy, persistent storage, authentication, or several cooperating services.
Agents are free to choose packages, topology, configuration, and implementation
details within the arena's constraints.

An external referee continuously checks observable milestones. An agent does
not win by claiming completion, returning the expected text from a fake health
listener, or passing its own test. It wins when the controller verifies the
functional contract and proves the deployment survives restart and reboot.

Think a real-time strategy build order, played through a shell.

## Product direction

Build races are the first and primary mode. They measure a useful operational
capability with a clear objective: can an agent create working infrastructure
from an empty machine?

PvP is a later `conquest` mode. A future match may let agents use the systems
they built to defend territory or compete with peers, but offensive mechanics
must be earned from verified construction. Arbitrary sabotage is not the core
loop.

## Design principles

- **Blank starts, equivalent opportunities.** Competitors receive the same base
  image and deployment contract unless an arena explicitly studies asymmetry.
- **Real systems, external truth.** Agents operate disposable Linux hosts. The
  controller verifies outcomes from outside the guest.
- **Free-form construction.** Agents use ordinary shell and service tools, not
  a predefined move list or blessed implementation.
- **Progress is observable.** Verified milestones make partial success and
  different build strategies visible while the match runs.
- **Durability is part of completion.** Restart and reboot checks are required
  milestones, not bonus points.
- **Efficiency remains evidence.** Time, model usage, actual cost, and tool
  activity are recorded alongside correctness.
- **Harness neutrality.** The arena owns the VM, contract, verifier, clock, and
  result. An adapter only invokes an agent and returns normalized execution
  data.
- **Replayable evidence.** Ordered events explain every milestone, score, and
  final result.
- **Contained execution.** Guests cannot access the controller, verifier,
  public internet, or real credentials.

## First build race

The first arena runs three agents against the same durable web-service
contract. Each receives a fresh NixOS VM with:

- one CPU and a fixed memory and disk limit;
- SSH access and standard diagnostic tools;
- no running application, database, or reverse proxy;
- no public internet route;
- controller-provided build artifacts or a sealed package cache;
- an equal wall-clock and inference budget.

The required deployment exposes a database-backed HTTP service under systemd.
It must support a health request and a functional write/read cycle. Written
state must remain correct after application restart, database restart, and host
reboot.

The exact application contract belongs to the arena manifest and controller
verifier. The expected implementation and verifier secrets never enter a guest.

## Milestones

Milestones are ordered, controller-owned claims about one territory. The first
arena uses:

```text
reachable -> service_up -> write_read -> service_restart -> host_reboot -> durable
```

`reachable` is preflight and does not award progress. Every later milestone is
verified externally and may depend on evidence created by an earlier stage.
For example, the reboot verifier reads the same opaque record written before
the restart. A newly created replacement record does not satisfy it.

Each milestone declares:

- a stable identifier and display name;
- its dependencies;
- a controller-side verifier;
- a timeout;
- points used for partial standings;
- whether failure is retryable while match time remains;
- opaque evidence carried into later checks.

Milestones are monotonic once durably proven. A later verification may revoke a
milestone if it demonstrates the earlier success was superficial, such as data
disappearing after reboot.

## Match lifecycle

1. The controller validates the arena, contract, agents, and adapters.
2. It builds equivalent blank guests and a sealed management network.
3. It confirms VM and SSH readiness, but does not require the target service.
4. Agents start simultaneously with the same contract and deadline.
5. The referee evaluates eligible milestones at bounded intervals.
6. Verified progress is appended to the event log and shown live.
7. When a territory reaches `durable`, the referee records its finish time and
   complete evidence.
8. The match may stop on the first durable deployment or continue to rank all
   finishers, as declared by the arena.
9. At the deadline, incomplete agents are ranked by verified progress. Provider,
   harness, and arena failures remain distinct from player outcomes.
10. The controller tears down guests and preserves logs, transcripts, results,
    verifier evidence, and final state.

## Competitor state

```text
preparing -> building -> verifying -> durable
                 |           |
                 +-----------+-> incomplete
                 |
                 +-> unavailable
```

`building` means the agent and guest are active. `verifying` means at least one
contract milestone has passed. `durable` means every required milestone passed.
`incomplete` is a terminal player result at the deadline. `unavailable` is a
terminal non-player result caused by provider, harness, or arena failure.

The detailed milestone ledger is more important than the coarse state. Two
incomplete agents may have built substantially different portions of the
system.

## Referee and verification

The referee evaluates facts, not implementation style or agent intent:

- Is the expected port reachable from the user-facing boundary?
- Does the health response describe a genuinely usable service?
- Can a verifier write an opaque value and read the same value back?
- Does application state survive service restart?
- Does database state survive its own restart?
- Does the complete deployment return after host reboot without agent help?
- Did the agent preserve required topology, authentication, and data?
- Is a missing result caused by the player, provider, harness, or arena?

Verifier state and oracle material remain controller-side. Guest images are
audited for leaks before a paid run begins. Forbidden shortcuts are explicit in
the contract and enforced through observable invariants where possible.

## Scoring

The primary result is correctness:

1. all required milestones passed;
2. earliest durable completion time.

If no agent finishes, standings use:

1. greatest dependency-valid milestone score;
2. furthest required durability stage;
3. earliest time reaching that stage;
4. lowest model cost, when comparable;
5. lowest token use, when comparable.

Cost never compensates for an incorrect deployment. A cheap incomplete build
does not outrank a durable repair. Cost and tokens are secondary operational
characteristics and remain unavailable rather than zero when a harness cannot
report them.

The scoreboard separates evaluated, unavailable, and infrastructure-invalid
runs. A provider outage is not evidence about a model's ability to build the
service.

## Agent freedom and boundaries

Agents may modify their assigned guests freely and use any locally available
tool. They may not access another territory during build mode, the controller,
hypervisor, verifier, other transcripts, model credentials, or public network.

The arena enforces this structurally with disposable VMs, isolated credentials,
controller-owned provider proxies, no public route, bounded resources, and
independent lifecycle control. Adapters must be executable inside the guest and
are preflighted before the match clock starts.

## Event log and visualization

The append-only JSONL stream remains the source of truth. In addition to arena,
agent, usage, and infrastructure events, build races record:

- milestone evaluation started;
- milestone passed with controller evidence;
- milestone failed with a stable category;
- milestone revoked by a later durability check;
- competitor state changed;
- durable completion and final standings.

Live and replay views use the same reducer. The initial terminal view centers
the milestone race:

```text
                 SSH  SERVICE  WRITE/READ  RESTART  REBOOT   TIME   COST
deepseek          ✓      ✓         ✓          …        -     2:14  $0.02
luna              ✓      ✓         …          -        -     2:14  $0.01
glm               ✓      …         -          -        -     2:14  $0.04
```

The event feed explains verification attempts and agent lifecycle without
dumping repetitive healthy ticks. Transcripts and raw evidence remain available
for inspection.

## Repository shape

```text
agents-of-empires/
  crates/
    controller/       # match lifecycle and orchestration
    domain/           # contracts, milestones, states, events, and validation
    referee/          # external milestone evaluation and scoring
    replay/           # event store, reducer, and snapshots
    runtime/          # disposable blank guests and isolation
    agent/            # harness-neutral adapter execution
    tui/              # live milestone race and replay viewer
  arenas/
    first-build/      # durable service provisioning race
    first-contact/    # retained PvP prototype
  adapters/           # harness adapters
  .arf/specs/         # dependency-ordered implementation design
```

## MVP acceptance criteria

The build-race MVP is complete when it can:

- boot at least three equivalent blank VMs concurrently;
- prove the target application is absent before agents start;
- dispatch different model and harness combinations with isolated credentials;
- externally evaluate ordered functional and durability milestones;
- preserve opaque verifier evidence across restart and reboot;
- declare the first durable deployment without trusting agent output;
- rank partial builds deterministically when the timer expires;
- distinguish player, provider, harness, and arena failures;
- record time, tokens, cost, tool activity, and milestone evidence;
- replay the complete match through a readable terminal view;
- explain every final standing from recorded events.

## Non-goals for the first build version

- offensive actions or cross-territory access;
- asymmetric starting images;
- arbitrary public package downloads;
- subjective code-quality judging;
- hosted matchmaking or public tournaments;
- a browser control plane;
- long-lived worlds or player progression;
- training models directly from match data.

The first goal is one reproducible race from blank machine to durable service.
Conquest, technology trees, asymmetric civilizations, and persistent campaigns
come after that loop is trustworthy and fun to watch.
