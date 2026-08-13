#!/usr/bin/env bash
set -euo pipefail
: "${OPENROUTER_API_KEY:=}"
root="$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-queue-credentials.XXXXXX")"
for entry in queue-one:queue-one-race queue-two:queue-two-race queue-three:queue-three-race; do
  territory="${entry%%:*}"
  password="${entry#*:}"
  printf 'OPENROUTER_API_KEY=%q\nAOE_SSH_PASSWORD=%q\n' "$OPENROUTER_API_KEY" "$password" >"$root/$territory.env"
  chmod 0600 "$root/$territory.env"
done
printf '%s\n' "$root"
