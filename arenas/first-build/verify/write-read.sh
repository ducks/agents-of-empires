#!/usr/bin/env bash
set -euo pipefail
record="record-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
value="opaque-$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
curl --silent --show-error --fail --max-time 3 -X PUT --data-binary "$value" "http://${AOE_HOST}:${AOE_SERVICE_PORT}/records/${record}" >/dev/null
actual="$(curl --silent --show-error --fail --max-time 3 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/records/${record}")"
[[ "$actual" == "$value" ]] || { echo "write/read value mismatch" >&2; exit 1; }
jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value}' >"$AOE_EVIDENCE_FILE"
