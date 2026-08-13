#!/usr/bin/env bash
set -euo pipefail
record="$(jq -r '."write-new".evidence.record' "$AOE_PREVIOUS_EVIDENCE")"
value="$(jq -r '."write-new".evidence.value' "$AOE_PREVIOUS_EVIDENCE")"
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
for _ in $(seq 1 40); do
  health="$(curl --silent --max-time 2 "$base/health" || true)"
  version="$(curl --silent --max-time 2 "$base/version" || true)"
  actual="$(curl --silent --max-time 2 "$base/records/$record" || true)"
  alpha="$(curl --silent --max-time 2 "$base/records/customer-alpha-73c" || true)"
  beta="$(curl --silent --max-time 2 "$base/records/customer-beta-a19" || true)"
  if [[ "$health" == ready && "$version" == v2 && "$actual" == "$value" && "$alpha" == alpha-original && "$beta" == beta-original ]]; then
    jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,reboot:true,version:"v2"}' >"$AOE_EVIDENCE_FILE"
    exit 0
  fi
  sleep 1
done
echo "v2 deployment or preserved records did not survive host reboot" >&2
exit 1
