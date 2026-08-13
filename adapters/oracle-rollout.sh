#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"
trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"
chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")
"${remote[@]}" 'install -d -m 0755 /opt/rollout
cat > /opt/rollout/v2.py <<'"'"'PY'"'"'
import sqlite3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
DB = "/var/lib/rollout/records.sqlite3"
def database():
    connection = sqlite3.connect(DB, timeout=10)
    connection.execute("CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value TEXT NOT NULL)")
    return connection
class Handler(BaseHTTPRequestHandler):
    def send(self, status, body=b""):
        self.send_response(status); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        if self.path == "/health": self.send(200, b"ready"); return
        if self.path == "/version": self.send(200, b"v2"); return
        if self.path.startswith("/records/"):
            connection = database(); row = connection.execute("SELECT value FROM records WHERE id=?", (self.path[9:],)).fetchone(); connection.close()
            self.send(404 if row is None else 200, b"" if row is None else row[0].encode()); return
        self.send(404)
    def do_PUT(self):
        if not self.path.startswith("/records/"): self.send(404); return
        value = self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode(); connection = database()
        connection.execute("INSERT INTO records(id,value) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET value=excluded.value", (self.path[9:], value))
        connection.commit(); connection.close(); self.send(204)
    def log_message(self, *_): pass
ThreadingHTTPServer(("127.0.0.1", 8082), Handler).serve_forever()
PY
chmod 0755 /opt/rollout/v2.py
systemctl reset-failed rollout-v2.service
systemctl restart rollout-v2.service
for _ in $(seq 1 30); do
  [[ "$(curl -fsS --max-time 1 http://127.0.0.1:8082/version 2>/dev/null || true)" == v2 ]] && break
  sleep .1
done
[[ "$(curl -fsS --max-time 1 http://127.0.0.1:8082/version)" == v2 ]]
printf "8082\n" > /var/lib/rollout/upstream.new
mv /var/lib/rollout/upstream.new /var/lib/rollout/upstream'
jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle performed an atomic stateful v2 cutover",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
