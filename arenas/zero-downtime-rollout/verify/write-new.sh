#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
record="rollout-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
value="opaque-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
status="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' --max-time 2 -X PUT --data-binary "$value" "$base/records/$record")"
[[ "$status" == 204 ]] || { echo "v2 PUT returned $status" >&2; exit 1; }
actual="$(curl --silent --show-error --fail --max-time 2 "$base/records/$record")"
[[ "$actual" == "$value" ]] || { echo "v2 write/read mismatch" >&2; exit 1; }
jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value}' >"$AOE_EVIDENCE_FILE"
