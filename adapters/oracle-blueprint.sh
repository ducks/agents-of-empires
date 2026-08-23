#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")
"${remote[@]}" 'install -d -m 0755 /opt/blueprint /var/lib/blueprint
cat > /opt/blueprint/store.py <<'"'"'PY'"'"'
import sqlite3
DB = "/var/lib/blueprint/jobs.sqlite3"
def connect():
    db = sqlite3.connect(DB, timeout=10)
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, payload BLOB NOT NULL, state TEXT NOT NULL)")
    db.commit()
    return db
connect().close()
PY
cat > /opt/blueprint/api.py <<'"'"'PY'"'"'
import sqlite3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from store import connect
class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body):
        self.send_response(status); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_GET(self):
        if self.path == "/health": self.reply(200, b"ready"); return
        if not self.path.startswith("/jobs/"): self.reply(404, b"missing"); return
        db = connect(); row = db.execute("SELECT payload,state FROM jobs WHERE id=?", (self.path[6:],)).fetchone(); db.close()
        if not row: self.reply(404, b"missing")
        elif row[1] == "pending": self.reply(409, b"pending")
        else: self.reply(200, bytes(row[0]))
    def do_PUT(self):
        if not self.path.startswith("/jobs/"): self.reply(404, b"missing"); return
        payload = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        db = connect()
        try: db.execute("INSERT INTO jobs(id,payload,state) VALUES(?,?,\"pending\")", (self.path[6:], payload)); db.commit(); self.reply(202, b"accepted")
        except sqlite3.IntegrityError: self.reply(409, b"duplicate")
        finally: db.close()
    def log_message(self, *_): pass
ThreadingHTTPServer(("127.0.0.1", 8081), Handler).serve_forever()
PY
cat > /opt/blueprint/edge.py <<'"'"'PY'"'"'
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
    def proxy(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0"))) if self.command == "PUT" else None
        conn = http.client.HTTPConnection("127.0.0.1", 8081, timeout=4)
        conn.request(self.command, self.path, body=body)
        response = conn.getresponse(); data = response.read(); conn.close()
        self.send_response(response.status); self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
    do_GET = proxy
    do_PUT = proxy
    def log_message(self, *_): pass
ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
PY
cat > /opt/blueprint/worker.py <<'"'"'PY'"'"'
import time
from store import connect
while True:
    db = connect(); db.execute("BEGIN IMMEDIATE")
    row = db.execute("SELECT id FROM jobs WHERE state=\"pending\" ORDER BY rowid LIMIT 1").fetchone()
    if row: db.execute("UPDATE jobs SET state=\"complete\" WHERE id=? AND state=\"pending\"", (row[0],)); db.commit()
    else: db.commit(); time.sleep(.1)
    db.close()
PY
cat > /opt/blueprint/edge <<'"'"'SH'"'"'
#!/bin/sh
exec /run/current-system/sw/bin/python3 /opt/blueprint/edge.py
SH
cat > /opt/blueprint/api <<'"'"'SH'"'"'
#!/bin/sh
exec /run/current-system/sw/bin/python3 /opt/blueprint/api.py
SH
cat > /opt/blueprint/worker <<'"'"'SH'"'"'
#!/bin/sh
exec /run/current-system/sw/bin/python3 /opt/blueprint/worker.py
SH
chmod 0755 /opt/blueprint/edge /opt/blueprint/api /opt/blueprint/worker
systemctl reset-failed blueprint-api.service blueprint-edge.service blueprint-worker.service
systemctl start blueprint-api.service blueprint-edge.service blueprint-worker.service'
jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle implemented the attached asynchronous blueprint",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
