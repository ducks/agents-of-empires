#!/usr/bin/env bash
set -euo pipefail

: "${OPENROUTER_API_KEY:?set OPENROUTER_API_KEY before preparing benchmark credentials}"
root="$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-infra-core.XXXXXX")"

for entry in \
  builder-one:builder-one-race \
  builder-two:builder-two-race \
  builder-three:builder-three-race \
  queue-one:queue-one-race \
  queue-two:queue-two-race \
  queue-three:queue-three-race \
  rollout-one:rollout-one-race \
  rollout-two:rollout-two-race \
  rollout-three:rollout-three-race \
  failover-one:failover-one-race \
  failover-two:failover-two-race \
  failover-three:failover-three-race
do
  territory="${entry%%:*}"
  password="${entry#*:}"
  printf 'OPENROUTER_API_KEY=%q\nAOE_SSH_PASSWORD=%q\n' \
    "$OPENROUTER_API_KEY" "$password" >"$root/$territory.env"
  chmod 0600 "$root/$territory.env"
done

printf '%s\n' "$root"
