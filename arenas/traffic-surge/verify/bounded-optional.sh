#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
work="$(mktemp -d)"
trap 'jobs -pr | xargs -r kill 2>/dev/null || true; rm -rf "$work"' EXIT
for index in $(seq 1 16); do
  (
    code="$(curl -s --max-time 2 -o "$work/body-$index" -w '%{http_code}' "$base/optional/bounded-$index" || true)"
    printf '%s\n' "$code" >"$work/code-$index"
  ) &
done
wait
served=0; shed=0
for index in $(seq 1 16); do
  code="$(cat "$work/code-$index")"
  case "$code" in
    200)
      [[ "$(cat "$work/body-$index")" == "optional:bounded-$index" ]]
      served=$((served + 1))
      ;;
    429|503) shed=$((shed + 1)) ;;
    *) echo "optional request $index was neither served nor deliberately shed: $code" >&2; exit 1 ;;
  esac
done
jq -n --argjson served "$served" --argjson shed "$shed" '{requests:16,served:$served,shed:$shed,bounded:true}' >"$AOE_EVIDENCE_FILE"
