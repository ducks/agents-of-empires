#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
[[ "$(curl -fsS --max-time 2 "$base/health")" == ready ]]
[[ "$(curl -fsS --max-time 2 "$base/role")" == primary ]]
[[ "$(curl -fsS --max-time 2 "$base/records/customer-red-4a7")" == red-original ]]
[[ "$(curl -fsS --max-time 2 "$base/records/customer-blue-8d2")" == blue-original ]]
[[ "$(curl -fsS --max-time 2 "$base/records/customer-green-f31")" == green-original ]]
jq -n '{reads:true,role:"primary",records:["customer-red-4a7","customer-blue-8d2","customer-green-f31"]}' >"$AOE_EVIDENCE_FILE"
