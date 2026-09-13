#!/usr/bin/env bash
set -euo pipefail

case "${1:-openrouter}" in
  openrouter) key_env=OPENROUTER_API_KEY ;;
  vercel) key_env=AI_GATEWAY_API_KEY ;;
  *) echo 'usage: prepare-infra-core-credentials.sh [openrouter|vercel]' >&2; exit 2 ;;
esac
[[ -n "${!key_env:-}" ]] || {
  echo "set ${key_env} before preparing credentials" >&2
  exit 2
}
umask 077
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
  printf '%s=%q\nAOE_SSH_PASSWORD=%q\n' \
    "$key_env" "${!key_env}" "$password" >"$root/$territory.env"
  chmod 0600 "$root/$territory.env"
done

printf '%s\n' "$root"
