#!/usr/bin/env bash
set -euo pipefail
root="$(mktemp -d "${TMPDIR:-/var/tmp}/aoe-hello-credentials.XXXXXX")"
for territory in builder-one builder-two builder-three; do
  printf 'AOE_SSH_PASSWORD=%q\n' "$territory-race" >"$root/$territory.env"
  chmod 0600 "$root/$territory.env"
done
printf '%s\n' "$root"
