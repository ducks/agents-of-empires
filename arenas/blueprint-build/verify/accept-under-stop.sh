#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
stopped=false
for _ in $(seq 1 10); do
  if env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl stop blueprint-worker.service'; then
    stopped=true; break
  fi
  sleep 0.5
done
[[ "$stopped" == true ]] || { echo "could not stop worker lifecycle unit" >&2; exit 1; }

jobs=()
payloads=()
for label in cedar cobalt saffron; do
  job="${label}-$(od -An -N6 -tx1 /dev/urandom | tr -d ' \n')"
  payload="opaque/${label}/$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
  response="$(mktemp)"; trap 'rm -f "$askpass" "$response"' EXIT
  status="$(curl --silent --show-error --output "$response" --write-out '%{http_code}' --max-time 3 -X PUT --data-binary "$payload" "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${job}")"
  [[ "$status" == 202 && "$(<"$response")" == accepted ]] || { echo "job acceptance contract failed" >&2; exit 1; }
  status="$(curl --silent --show-error --output "$response" --write-out '%{http_code}' --max-time 3 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${job}")"
  [[ "$status" == 409 && "$(<"$response")" == pending ]] || { echo "job did not remain pending while worker was stopped" >&2; exit 1; }
  jobs+=("$job"); payloads+=("$payload")
  rm -f "$response"
done
jq -n \
  --argjson jobs "$(printf '%s\n' "${jobs[@]}" | jq -R . | jq -s .)" \
  --argjson payloads "$(printf '%s\n' "${payloads[@]}" | jq -R . | jq -s .)" \
  '{jobs:$jobs,payloads:$payloads,worker_stopped:true}' >"$AOE_EVIDENCE_FILE"
