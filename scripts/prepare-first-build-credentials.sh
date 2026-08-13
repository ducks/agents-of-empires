#!/usr/bin/env bash
set -euo pipefail
root="$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-build-credentials.XXXXXX")"
for entry in builder-one:builder-one-race builder-two:builder-two-race builder-three:builder-three-race; do
  territory="${entry%%:*}"
  password="${entry#*:}"
  printf 'AOE_SSH_PASSWORD=%q\n' "$password" >"$root/$territory.env"
  chmod 0600 "$root/$territory.env"
done
printf '%s\n' "$root"
