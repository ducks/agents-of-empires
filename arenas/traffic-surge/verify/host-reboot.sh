#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
record="$(jq -r '."restart-under-load".evidence.record' "$AOE_PREVIOUS_EVIDENCE")"
value="$(jq -r '."restart-under-load".evidence.value' "$AOE_PREVIOUS_EVIDENCE")"
for _ in $(seq 1 35); do
  if [[ "$(curl -s --max-time 2 "$base/health" || true)" == ready ]] && \
     [[ "$(curl -s --max-time 2 "$base/priority/$record" || true)" == "$value" ]] && \
     [[ "$(curl -s --max-time 2 "$base/priority/history-amber-a91" || true)" == ledger-alpha ]] && \
     [[ "$(curl -s --max-time 2 "$base/priority/history-cobalt-f73" || true)" == ledger-beta ]] && \
     [[ "$(curl -s --max-time 2 "$base/priority/history-umber-2d4" || true)" == ledger-gamma ]]; then
    jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,historical_records:3,reboot:true}' >"$AOE_EVIDENCE_FILE"
    exit 0
  fi
  sleep 1
done
echo "priority state or traffic service did not survive reboot" >&2
exit 1
