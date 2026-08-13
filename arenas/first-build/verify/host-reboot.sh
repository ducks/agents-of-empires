#!/usr/bin/env bash
set -euo pipefail
record="$(jq -r '."write-read".evidence.record' "$AOE_PREVIOUS_EVIDENCE")"
value="$(jq -r '."write-read".evidence.value' "$AOE_PREVIOUS_EVIDENCE")"
for _ in $(seq 1 30); do
  health="$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health" || true)"
  actual="$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/records/${record}" || true)"
  [[ "$health" == "ready" && "$actual" == "$value" ]] && { jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,reboot:true}' >"$AOE_EVIDENCE_FILE"; exit 0; }
  sleep 1
done
echo "deployment or opaque record did not survive host reboot" >&2
exit 1
