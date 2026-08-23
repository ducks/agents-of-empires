#!/usr/bin/env bash
set -euo pipefail
mapfile -t jobs < <(jq -r '."service-restart".evidence.jobs[]' "$AOE_PREVIOUS_EVIDENCE")
mapfile -t payloads < <(jq -r '."service-restart".evidence.payloads[]' "$AOE_PREVIOUS_EVIDENCE")
for attempt in $(seq 1 35); do
  good=1
  [[ "$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health" || true)" == ready ]] || good=0
  for index in "${!jobs[@]}"; do
    response="$(mktemp)"
    status="$(curl --silent --output "$response" --write-out '%{http_code}' --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${jobs[$index]}" || true)"
    [[ "$status" == 200 && "$(<"$response")" == "${payloads[$index]}" ]] || good=0
    rm -f "$response"
  done
  [[ "$good" == 1 ]] && { jq -n --argjson jobs "$(printf '%s\n' "${jobs[@]}" | jq -R . | jq -s .)" '{jobs:$jobs,reboot:true}' >"$AOE_EVIDENCE_FILE"; exit 0; }
  sleep 1
done
echo "blueprint deployment or exact results did not survive reboot" >&2
exit 1
