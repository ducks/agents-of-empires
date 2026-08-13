#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl restart failover-replica.service failover-proxy.service'
record="$(jq -r '."writes-restored".evidence.record' "$AOE_PREVIOUS_EVIDENCE")"
value="$(jq -r '."writes-restored".evidence.value' "$AOE_PREVIOUS_EVIDENCE")"
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
for _ in $(seq 1 20); do
  if [[ "$(curl -s --max-time 1 "$base/role")" == primary && "$(curl -s --max-time 1 "$base/records/$record")" == "$value" ]]; then
    jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,restart:true}' >"$AOE_EVIDENCE_FILE"; exit 0
  fi
  sleep 0.5
done
echo "promoted topology did not survive service restart" >&2; exit 1
