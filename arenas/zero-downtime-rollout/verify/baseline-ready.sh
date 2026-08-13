#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
health="$(curl --silent --show-error --fail --max-time 2 "$base/health")"
version="$(curl --silent --show-error --fail --max-time 2 "$base/version")"
alpha="$(curl --silent --show-error --fail --max-time 2 "$base/records/customer-alpha-73c")"
beta="$(curl --silent --show-error --fail --max-time 2 "$base/records/customer-beta-a19")"
[[ "$health" == ready && "$version" == v1 && "$alpha" == alpha-original && "$beta" == beta-original ]] || {
  echo "live v1 baseline or original records are not intact" >&2
  exit 1
}
audit="$(dirname "$AOE_EVIDENCE_FILE")/continuity-audit"
mkdir -p "$audit"
: >"$audit/failures"
: >"$audit/samples"
rm -f "$audit/saw-v2"
nohup "$(dirname "$0")/continuity-monitor.sh" "$base" "$audit" </dev/null >/dev/null 2>&1 &
printf '%s\n' "$!" >"$audit/pid"
jq -n '{baseline:"v1",records:["customer-alpha-73c","customer-beta-a19"]}' >"$AOE_EVIDENCE_FILE"
