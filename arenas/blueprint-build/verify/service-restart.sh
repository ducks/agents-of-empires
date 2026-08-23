#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
restarted=false
for _ in $(seq 1 10); do
  if env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl restart blueprint-api.service blueprint-edge.service blueprint-worker.service'; then
    restarted=true; break
  fi
  sleep 0.5
done
[[ "$restarted" == true ]] || { echo "could not restart blueprint lifecycle units" >&2; exit 1; }

mapfile -t jobs < <(jq -r '."recover-blueprint".evidence.jobs[]' "$AOE_PREVIOUS_EVIDENCE")
mapfile -t payloads < <(jq -r '."recover-blueprint".evidence.payloads[]' "$AOE_PREVIOUS_EVIDENCE")
fresh_job="restart-$(od -An -N6 -tx1 /dev/urandom | tr -d ' \n')"
fresh_payload="opaque/restart/$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
for attempt in $(seq 1 20); do
  [[ "$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health" || true)" == ready ]] && break
  sleep 1
done
[[ "$(curl --silent --show-error --max-time 3 -X PUT --data-binary "$fresh_payload" "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${fresh_job}")" == accepted ]]
jobs+=("$fresh_job"); payloads+=("$fresh_payload")
for attempt in $(seq 1 20); do
  good=1
  for index in "${!jobs[@]}"; do
    response="$(mktemp)"
    status="$(curl --silent --show-error --output "$response" --write-out '%{http_code}' --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${jobs[$index]}" || true)"
    [[ "$status" == 200 && "$(<"$response")" == "${payloads[$index]}" ]] || good=0
    rm -f "$response"
  done
  [[ "$good" == 1 ]] && { jq -n --argjson jobs "$(printf '%s\n' "${jobs[@]}" | jq -R . | jq -s .)" --argjson payloads "$(printf '%s\n' "${payloads[@]}" | jq -R . | jq -s .)" '{jobs:$jobs,payloads:$payloads,restarted:true}' >"$AOE_EVIDENCE_FILE"; exit 0; }
  sleep 1
done
echo "service restart lost historical or fresh work" >&2
exit 1
