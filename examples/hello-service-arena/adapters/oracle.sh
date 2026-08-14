#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")

"${remote[@]}" 'install -d -m 0755 /opt/arena
cat > /opt/arena/app.py <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/health": self.send_error(404); return
        body = b"ready"
        self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *_): pass
ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
PY
systemctl reset-failed arena-app.service
systemctl restart arena-app.service'

jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle deployed hello service",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
