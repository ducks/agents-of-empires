#!/usr/bin/env bash
set -euo pipefail
: "${OPENROUTER_API_KEY:=}"
root="$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-blueprint-credentials.XXXXXX")"
for entry in blueprint-one:blueprint-one-race blueprint-two:blueprint-two-race blueprint-three:blueprint-three-race; do
  territory="${entry%%:*}"
  password="${entry#*:}"
  printf 'OPENROUTER_API_KEY=%q\nAOE_SSH_PASSWORD=%q\n' "$OPENROUTER_API_KEY" "$password" >"$root/$territory.env"
  chmod 0600 "$root/$territory.env"
done
printf '%s\n' "$root"
