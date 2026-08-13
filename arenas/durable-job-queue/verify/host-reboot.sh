#!/usr/bin/env bash
set -euo pipefail
job="$(jq -r '."process-new".evidence.job' "$AOE_PREVIOUS_EVIDENCE")"
payload="$(jq -r '."process-new".evidence.payload' "$AOE_PREVIOUS_EVIDENCE")"
for _ in $(seq 1 30); do
  health="$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health" || true)"
  body="$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${job}" || true)"
  if [[ "$health" == ready ]] && jq -e --arg payload "$payload" '.status=="completed" and .result==( "processed:"+$payload ) and .attempts==1' <<<"$body" >/dev/null 2>&1; then
    jq -n --arg job "$job" --arg payload "$payload" '{job:$job,payload:$payload,reboot:true}' >"$AOE_EVIDENCE_FILE"; exit 0
  fi
  sleep 1
done
echo "queue deployment or completed job did not survive reboot" >&2
exit 1
