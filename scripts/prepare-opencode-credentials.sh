#!/usr/bin/env bash
set -euo pipefail

# Only the explicitly exported key is included; no auth caches are searched.
: "${OPENCODE_API_KEY:?export OPENCODE_API_KEY before preparing OpenCode credentials}"
credential_root="$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-opencode.XXXXXX")"
for entry in \
  builder-one:builder-one-race builder-two:builder-two-race builder-three:builder-three-race \
  queue-one:queue-one-race queue-two:queue-two-race queue-three:queue-three-race \
  rollout-one:rollout-one-race rollout-two:rollout-two-race rollout-three:rollout-three-race \
  failover-one:failover-one-race failover-two:failover-two-race failover-three:failover-three-race
do
  printf 'OPENCODE_API_KEY=%q\nAOE_SSH_PASSWORD=%q\n' "$OPENCODE_API_KEY" "${entry#*:}" \
    >"$credential_root/${entry%%:*}.env"
  chmod 0600 "$credential_root/${entry%%:*}.env"
done
printf '%s\n' "$credential_root"
