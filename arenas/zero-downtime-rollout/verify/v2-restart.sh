#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl restart rollout-v2.service'
record="$(jq -r '."write-new".evidence.record' "$AOE_PREVIOUS_EVIDENCE")"
value="$(jq -r '."write-new".evidence.value' "$AOE_PREVIOUS_EVIDENCE")"
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
for _ in $(seq 1 20); do
  health="$(curl --silent --max-time 1 "$base/health" || true)"
  version="$(curl --silent --max-time 1 "$base/version" || true)"
  actual="$(curl --silent --max-time 1 "$base/records/$record" || true)"
  if [[ "$health" == ready && "$version" == v2 && "$actual" == "$value" ]]; then
    jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,v2_restart:true}' >"$AOE_EVIDENCE_FILE"
    exit 0
  fi
  sleep 0.5
done
echo "v2 or opaque record did not recover after service restart" >&2
exit 1
