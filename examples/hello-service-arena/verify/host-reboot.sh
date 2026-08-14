#!/usr/bin/env bash
set -euo pipefail
for _ in $(seq 1 20); do
  body="$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health" || true)"
  if [[ "$body" == "ready" ]]; then
    jq -n '{host_reboot:true}' >"$AOE_EVIDENCE_FILE"
    exit 0
  fi
  sleep 0.5
done
echo "service did not recover after host reboot" >&2
exit 1
