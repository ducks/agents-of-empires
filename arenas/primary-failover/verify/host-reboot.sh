#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}")
record="$(jq -r '."writes-restored".evidence.record' "$AOE_PREVIOUS_EVIDENCE")"
value="$(jq -r '."writes-restored".evidence.value' "$AOE_PREVIOUS_EVIDENCE")"
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
for _ in $(seq 1 40); do
  health="$(curl -s --max-time 2 "$base/health" || true)"
  role="$(curl -s --max-time 2 "$base/role" || true)"
  actual="$(curl -s --max-time 2 "$base/records/$record" || true)"
  red="$(curl -s --max-time 2 "$base/records/customer-red-4a7" || true)"
  blue="$(curl -s --max-time 2 "$base/records/customer-blue-8d2" || true)"
  green="$(curl -s --max-time 2 "$base/records/customer-green-f31" || true)"
  if [[ "$health" == ready && "$role" == primary && "$actual" == "$value" && "$red" == red-original && "$blue" == blue-original && "$green" == green-original ]] && \
    "${remote[@]}" '[[ -e /var/lib/failover/primary.failed ]] && [[ -e /var/lib/failover/primary.fenced ]] && [[ "$(cat /var/lib/failover/upstream)" == 8082 ]] && ! ss -lnt | grep -q ":8081 "' 2>/dev/null; then
    jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,reboot:true,role:"primary",old_primary:"fenced"}' >"$AOE_EVIDENCE_FILE"; exit 0
  fi
  sleep 1
done
echo "promoted primary, fencing, or records did not survive reboot" >&2; exit 1
