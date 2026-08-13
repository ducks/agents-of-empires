#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_HOST}" 'systemctl restart queue-worker.service'
job="$(jq -r '."process-new".evidence.job' "$AOE_PREVIOUS_EVIDENCE")"
payload="$(jq -r '."process-new".evidence.payload' "$AOE_PREVIOUS_EVIDENCE")"
body="$(curl --silent --show-error --fail --max-time 3 "http://${AOE_HOST}:${AOE_SERVICE_PORT}/jobs/${job}")"
jq -e --arg payload "$payload" '.status=="completed" and .result==( "processed:"+$payload ) and .attempts==1' <<<"$body" >/dev/null
jq -n --arg job "$job" --arg payload "$payload" '{job:$job,payload:$payload,worker_restart:true}' >"$AOE_EVIDENCE_FILE"
