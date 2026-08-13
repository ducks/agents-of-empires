#!/usr/bin/env bash
set -euo pipefail

: "${OPENROUTER_API_KEY:?set OPENROUTER_API_KEY before preparing match credentials}"
output="${1:-$(mktemp -d "${TMPDIR:-/var/tmp}/agents-of-empires-credentials.XXXXXX")}"
mkdir -p "$output"
chmod 0700 "$output"

write_credential() {
  local territory="$1"
  local password="$2"
  local path="${output}/${territory}.env"
  umask 077
  printf 'OPENROUTER_API_KEY=%q\nAOE_SSH_PASSWORD=%q\n' \
    "$OPENROUTER_API_KEY" "$password" >"$path"
  chmod 0600 "$path"
}

write_credential gatekeeper gatekeeper-first-contact
write_credential archivist archivist-first-contact
write_credential courier courier-first-contact
printf '%s\n' "$output"
