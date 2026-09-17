# First harness exhibition review

Evidence: `matches/harness-exhibition-20260915-225225` (unchanged).

term-llm won at 45.216 seconds. The external referee passed service-up at
30.870s, write/read at 30.915s, service restart at 31.644s, and host reboot at
45.216s. The same opaque record survived restart and reboot. All seats were
configured for OpenRouter GPT-5.6 Luna, high reasoning, one vCPU and 768 MiB.
This establishes configured parity, not an identical upstream backend.

## Claux disconnect

Claux's retained tool trace shows a deployed service and successful self-tests
for health, binary record round-trip, and service-restart persistence. The last
completed commands inspect unit enablement, persistent storage, and logs.
The trace continues past 61 seconds relative to its own harness start; the
controller had frozen scoring at 45.216 seconds and allowed a 30-second drain.

Its guest console records an orderly shutdown followed by `reboot: Restarting
system`. The SSH adapter reports exit 255. There is no controller-owned
`referee-reboot` marker for this seat and no referee reboot event for it.
Neither the final transcript nor the live transcript includes a reboot command.
The available logs therefore confirm a reboot/disconnect but do not establish
the initiator. Do not attribute it to OOM, the referee, or the model without
additional evidence. The final failure label is a conservative harness error,
not proof that the harness caused the reboot. The winner is unaffected.

OpenCode was still investigating a refused connection after deploying its
service. It exhausted the drain and was recorded as outraced. Its missing
terminal adapter result does not imply provider failure.

## Accounting corrections

The raw adapter cost values for OpenCode and term-llm are null. Replay previously
added null as zero, and the TUI formatted that recorded subtotal as a total.
Replay now retains cost completeness separately; live rows, match report rows
and totals, and cup summaries distinguish unknown/partial cost from true zero.
Subscription routes retain their subscription label. Legacy cup summaries
without completeness metadata are conservatively labeled as recorded subtotals.

All three adapters now normalize input to fresh + cache-read + cache-write
tokens. Each harness's categories are disjoint; output is added separately by
the presentation layer. Final cumulative stats replace checkpoint sums. Adapter
hashes change to mark this accounting cohort; old logs are not rewritten.

For reference, recomputing from the retained raw traces gives these totals:

| Harness | Total input including cache | Output | Combined |
| --- | ---: | ---: | ---: |
| Claux | 81,754 | 4,582 | 86,336 |
| OpenCode | 87,893 | 2,390 | 90,283 |
| term-llm | 15,242 | 2,312 | 17,554 |

These are captured usage, not synchronized race-only totals: the losing
harnesses continued during the post-match drain. Unknown dollar charges cannot
be reconstructed from tokens alone. The original terminal screenshot also
precedes final drain accounting; Claux's final recorded spend is $0.009586.
