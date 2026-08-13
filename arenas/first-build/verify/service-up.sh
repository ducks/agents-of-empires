#!/usr/bin/env bash
set -euo pipefail
body="$(curl --silent --show-error --fail --max-time 3 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health")"
[[ "$body" == "ready" ]] || { echo "health body was not ready" >&2; exit 1; }
jq -n --arg body "$body" '{health:$body}' >"$AOE_EVIDENCE_FILE"
