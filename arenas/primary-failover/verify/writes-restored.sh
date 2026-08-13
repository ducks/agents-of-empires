#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
record="post-failover-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
value="opaque-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
status="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 -X PUT --data-binary "$value" "$base/records/$record")"
[[ "$status" == 204 ]] || { echo "promoted primary PUT returned $status" >&2; exit 1; }
[[ "$(curl -fsS --max-time 2 "$base/records/$record")" == "$value" ]]
jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,writes:true}' >"$AOE_EVIDENCE_FILE"
