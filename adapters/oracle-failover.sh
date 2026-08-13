#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")
"${remote[@]}" '[[ -e /var/lib/failover/primary.failed ]] &&
[[ "$(cat /var/lib/failover/replica.role)" == replica ]] &&
! ss -lnt | grep -q ":8081 " &&
touch /var/lib/failover/primary.fenced &&
printf "primary\n" > /var/lib/failover/replica.role.new &&
mv /var/lib/failover/replica.role.new /var/lib/failover/replica.role &&
[[ "$(curl -fsS http://127.0.0.1:8082/role)" == primary ]] &&
printf "8082\n" > /var/lib/failover/upstream.new &&
mv /var/lib/failover/upstream.new /var/lib/failover/upstream'
jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle promoted replica, fenced failed primary, and switched traffic",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
