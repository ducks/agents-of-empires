#!/usr/bin/env bash
set -euo pipefail
job="fresh-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
payload="opaque-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
status="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' --max-time 3 -X POST --data-binary "$payload" "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${job}")"
[[ "$status" == 202 ]] || { echo "POST returned $status" >&2; exit 1; }
for _ in $(seq 1 30); do
  body="$(curl --silent --show-error --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${job}" || true)"
  jq -e --arg id "$job" --arg payload "$payload" '.id==$id and .payload==$payload and .status=="completed" and .result==( "processed:"+$payload ) and .attempts==1' <<<"$body" >/dev/null 2>&1 && { jq -n --arg job "$job" --arg payload "$payload" '{job:$job,payload:$payload,completed:true}' >"$AOE_EVIDENCE_FILE"; exit 0; }
  sleep 1
done
echo "new job did not complete exactly once" >&2
exit 1
