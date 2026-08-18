#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
work="$(mktemp -d)"
trap 'jobs -pr | xargs -r kill 2>/dev/null || true; rm -rf "$work"' EXIT
for index in $(seq 1 8); do
  curl -s --max-time 5 "$base/optional/first-$index" >"$work/optional-$index" &
done
sleep 0.1
record="surge-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
value="priority-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
status="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 1.5 -X PUT --data-binary "$value" "$base/priority/$record")"
[[ "$status" == 204 ]] || { echo "priority PUT under first surge returned $status" >&2; exit 1; }
[[ "$(curl -fsS --max-time 1.5 "$base/priority/$record")" == "$value" ]]
jq -n --arg record "$record" --arg value "$value" '{record:$record,value:$value,optional_pressure:8}' >"$AOE_EVIDENCE_FILE"
