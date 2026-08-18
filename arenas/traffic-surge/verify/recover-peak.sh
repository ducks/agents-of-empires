#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
work="$(mktemp -d)"
trap 'jobs -pr | xargs -r kill 2>/dev/null || true; rm -rf "$work"' EXIT
for index in $(seq 1 24); do
  curl -s --max-time 4 -o /dev/null "$base/optional/peak-$index" &
done
sleep 0.1
records='[]'
for index in $(seq 1 4); do
  record="peak-${index}-$(od -An -N6 -tx1 /dev/urandom | tr -d ' \n')"
  value="accepted-${index}-$(od -An -N10 -tx1 /dev/urandom | tr -d ' \n')"
  code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 -X PUT --data-binary "$value" "$base/priority/$record")"
  [[ "$code" == 204 ]] || { echo "peak priority PUT $index returned $code" >&2; exit 1; }
  records="$(jq -c --arg id "$record" --arg value "$value" '. + [{id:$id,value:$value}]' <<<"$records")"
done
while IFS=$'\t' read -r record value; do
  [[ "$(curl -fsS --max-time 2 "$base/priority/$record")" == "$value" ]]
done < <(jq -r '.[] | [.id,.value] | @tsv' <<<"$records")
jq -n --argjson records "$records" '{records:$records,peak_optional_pressure:24,recoverable:true}' >"$AOE_EVIDENCE_FILE"
