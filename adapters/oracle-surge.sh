#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")
"${remote[@]}" 'cat > /opt/traffic-surge/app.py <<'"'"'PY'"'"'
#!/usr/bin/env python3
import sqlite3
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DB = "/var/lib/traffic-surge/priority.sqlite3"
optional_slots = threading.BoundedSemaphore(4)

def db():
    connection = sqlite3.connect(DB, timeout=10)
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("CREATE TABLE IF NOT EXISTS priority (id TEXT PRIMARY KEY, value BLOB NOT NULL)")
    return connection

def initialize():
    connection = db()
    for record, value in [("history-amber-a91", b"ledger-alpha"), ("history-cobalt-f73", b"ledger-beta"), ("history-umber-2d4", b"ledger-gamma")]:
        connection.execute("INSERT OR IGNORE INTO priority VALUES (?, ?)", (record, value))
    connection.commit(); connection.close()

class Handler(BaseHTTPRequestHandler):
    def reply(self, status, body=b""):
        self.send_response(status); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self.reply(200, b"ready")
        elif self.path.startswith("/priority/"):
            connection = db(); row = connection.execute("SELECT value FROM priority WHERE id = ?", (self.path[10:],)).fetchone(); connection.close()
            self.reply(200, row[0]) if row else self.reply(404)
        elif self.path.startswith("/optional/"):
            if not optional_slots.acquire(blocking=False):
                self.reply(503); return
            try:
                key = self.path[10:]; time.sleep(0.15); self.reply(200, ("optional:" + key).encode())
            finally:
                optional_slots.release()
        else:
            self.reply(404)

    def do_PUT(self):
        if not self.path.startswith("/priority/"):
            self.reply(404); return
        value = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        connection = db(); connection.execute("INSERT OR REPLACE INTO priority VALUES (?, ?)", (self.path[10:], value)); connection.commit(); connection.close()
        self.reply(204)

    def log_message(self, *_): pass

initialize()
ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
PY
chmod 0755 /opt/traffic-surge/app.py
systemctl restart traffic-surge.service'
jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle bounded optional load and protected durable priority traffic",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
