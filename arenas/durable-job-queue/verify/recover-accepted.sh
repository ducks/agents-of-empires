#!/usr/bin/env bash
set -euo pipefail
jobs=(accepted-alpha-7d3 accepted-beta-91e accepted-gamma-c42)
payloads=(alpha beta gamma)
for attempt in $(seq 1 30); do
  good=1
  for index in 0 1 2; do
    body="$(curl --silent --show-error --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${jobs[$index]}" || true)"
    jq -e --arg id "${jobs[$index]}" --arg payload "${payloads[$index]}" '.id==$id and .payload==$payload and .status=="completed" and .result==( "processed:"+$payload ) and .attempts==1' <<<"$body" >/dev/null 2>&1 || good=0
  done
  [[ "$good" == 1 ]] && { jq -n --argjson jobs "$(printf '%s\n' "${jobs[@]}" | jq -R . | jq -s .)" '{accepted_jobs:$jobs,recovered:true}' >"$AOE_EVIDENCE_FILE"; exit 0; }
  sleep 1
done
echo "pre-existing accepted jobs were not recovered exactly once" >&2
exit 1
