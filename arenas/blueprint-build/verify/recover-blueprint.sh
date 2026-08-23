#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
started=false
for _ in $(seq 1 10); do
  if env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl start blueprint-worker.service'; then
    started=true; break
  fi
  sleep 0.5
done
[[ "$started" == true ]] || { echo "could not start worker lifecycle unit" >&2; exit 1; }

mapfile -t jobs < <(jq -r '."accept-under-stop".evidence.jobs[]' "$AOE_PREVIOUS_EVIDENCE")
mapfile -t payloads < <(jq -r '."accept-under-stop".evidence.payloads[]' "$AOE_PREVIOUS_EVIDENCE")
for attempt in $(seq 1 30); do
  good=1
  for index in "${!jobs[@]}"; do
    response="$(mktemp)"
    status="$(curl --silent --show-error --output "$response" --write-out '%{http_code}' --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${jobs[$index]}" || true)"
    [[ "$status" == 200 && "$(<"$response")" == "${payloads[$index]}" ]] || good=0
    rm -f "$response"
  done
  [[ "$good" == 1 ]] && { jq -n --argjson jobs "$(printf '%s\n' "${jobs[@]}" | jq -R . | jq -s .)" --argjson payloads "$(printf '%s\n' "${payloads[@]}" | jq -R . | jq -s .)" '{jobs:$jobs,payloads:$payloads,recovered:true}' >"$AOE_EVIDENCE_FILE"; exit 0; }
  sleep 1
done
echo "queued jobs were not recovered with their exact bytes" >&2
exit 1
