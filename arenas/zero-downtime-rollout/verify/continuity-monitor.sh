#!/usr/bin/env bash
set -euo pipefail
base="$1"
audit="$2"
# Sample for the full match, not a fixed sample count (which expired early).
# Bound orphan lifetime even if the controller is interrupted or a seat never cuts over.
duration="${AOE_MATCH_DURATION_SECONDS:-900}"
[[ "$duration" =~ ^[0-9]+$ ]] || exit 2
deadline=$((SECONDS + duration + 120))
while (( SECONDS < deadline )); do
  health="$(curl --silent --max-time 1 "$base/health" || true)"
  version="$(curl --silent --max-time 1 "$base/version" || true)"
  alpha="$(curl --silent --max-time 1 "$base/records/customer-alpha-73c" || true)"
  beta="$(curl --silent --max-time 1 "$base/records/customer-beta-a19" || true)"
  printf '%s\t%s\t%s\t%s\n' "$health" "$version" "$alpha" "$beta" >>"$audit/samples"
  [[ "$health" == ready ]] || printf 'health=%q\n' "$health" >>"$audit/failures"
  [[ "$version" == v1 || "$version" == v2 ]] || printf 'version=%q\n' "$version" >>"$audit/failures"
  [[ "$alpha" == alpha-original ]] || printf 'alpha=%q\n' "$alpha" >>"$audit/failures"
  [[ "$beta" == beta-original ]] || printf 'beta=%q\n' "$beta" >>"$audit/failures"
  [[ "$version" == v2 ]] && touch "$audit/saw-v2"
  sleep 0.05
done
