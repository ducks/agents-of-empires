#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; work="$(mktemp -d)"
trap 'jobs -pr | xargs -r kill 2>/dev/null || true; rm -f "$askpass"; rm -rf "$work"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
(
  for index in $(seq 1 30); do
    curl -s --max-time 1 -o /dev/null "$base/optional/restart-$index" || true
  done
) &
env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl restart traffic-surge.service'
for _ in $(seq 1 20); do
  [[ "$(curl -s --max-time 1 "$base/health" || true)" == ready ]] && break
  sleep 0.25
done
record="restart-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
value="durable-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 -X PUT --data-binary "$value" "$base/priority/$record")"
[[ "$code" == 204 ]]
[[ "$(curl -fsS --max-time 2 "$base/priority/$record")" == "$value" ]]
jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,restart_under_load:true}' >"$AOE_EVIDENCE_FILE"
