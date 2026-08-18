#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
[[ "$(curl -fsS --max-time 2 "$base/health")" == ready ]]
for pair in history-amber-a91:ledger-alpha history-cobalt-f73:ledger-beta history-umber-2d4:ledger-gamma; do
  id="${pair%%:*}"; value="${pair#*:}"
  [[ "$(curl -fsS --max-time 2 "$base/priority/$id")" == "$value" ]]
done
jq -n '{health:"ready",historical_records:3}' >"$AOE_EVIDENCE_FILE"
