#!/usr/bin/env bash
set -euo pipefail
base="http://${AOE_HOST}:${AOE_SERVICE_PORT}"
audit="$(dirname "$AOE_EVIDENCE_FILE")/continuity-audit"
for _ in $(seq 1 500); do
  [[ -e "$audit/saw-v2" ]] && break
  sleep 0.2
done
[[ -e "$audit/saw-v2" ]] || { echo "public endpoint never cut over to v2" >&2; exit 1; }
sleep 0.5
kill "$(cat "$audit/pid")" 2>/dev/null || true
samples="$(wc -l <"$audit/samples")"
failures="$(wc -l <"$audit/failures")"
[[ "$samples" -ge 3 ]] || { echo "continuity monitor recorded only $samples samples" >&2; exit 1; }
[[ "$failures" -eq 0 ]] || { echo "observed $failures failed public checks during cutover" >&2; exit 1; }
for _ in $(seq 1 10); do
  [[ "$(curl --silent --max-time 1 "$base/version" || true)" == v2 ]] || {
    echo "public endpoint did not remain on v2" >&2
    exit 1
  }
  sleep 0.1
done
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" \
  '[[ "$(cat /var/lib/rollout/upstream)" == 8082 ]] && systemctl is-active --quiet rollout-v1.service rollout-v2.service rollout-proxy.service'
jq -n --argjson samples "$samples" '{version:"v2",uninterrupted:true,samples:$samples}' >"$AOE_EVIDENCE_FILE"
