#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh \
  -p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password \
  -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  "root@$AOE_HOST" 'systemctl restart arena-app.service'
for _ in $(seq 1 20); do
  body="$(curl --silent --max-time 2 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/health" || true)"
  if [[ "$body" == "ready" ]]; then
    jq -n '{service_restart:true}' >"$AOE_EVIDENCE_FILE"
    exit 0
  fi
  sleep 0.5
done
echo "service did not recover after restart" >&2
exit 1
