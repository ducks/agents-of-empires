#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}")
"${remote[@]}" '[[ -e /var/lib/failover/primary.failed ]] && [[ -e /var/lib/failover/primary.fenced ]] && [[ "$(cat /var/lib/failover/upstream)" == 8082 ]] && [[ "$(cat /var/lib/failover/replica.role)" == primary ]] && ! ss -lnt | grep -q ":8081 "'
jq -n '{old_primary:"fenced",old_port:8081,promoted_port:8082}' >"$AOE_EVIDENCE_FILE"
