#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")

"${remote[@]}" 'install -d -m 0755 /opt/builder /var/lib/builder
cat > /opt/builder/app.py <<'PY'
#!/usr/bin/env python3
import sqlite3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
DB = "/var/lib/builder/records.sqlite3"
def db():
    connection = sqlite3.connect(DB)
    connection.execute("CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value BLOB NOT NULL)")
    return connection
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            body = b"ready"
        elif self.path.startswith("/records/"):
            row = db().execute("SELECT value FROM records WHERE id = ?", (self.path[9:],)).fetchone()
            if row is None:
                self.send_error(404); return
            body = row[0]
        else:
            self.send_error(404); return
        self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_PUT(self):
        if not self.path.startswith("/records/"):
            self.send_error(404); return
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        connection = db(); connection.execute("INSERT OR REPLACE INTO records VALUES (?, ?)", (self.path[9:], body)); connection.commit(); connection.close()
        self.send_response(204); self.end_headers()
    def log_message(self, *_): pass
db().close()
ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
PY
chmod 0755 /opt/builder/app.py
systemctl reset-failed builder-app.service
systemctl restart builder-app.service'

jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle deployed durable service",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
