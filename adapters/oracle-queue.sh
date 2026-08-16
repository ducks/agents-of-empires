#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")
"${remote[@]}" 'install -d -m 0755 /opt/job-queue /var/lib/job-queue
cat > /opt/job-queue/common.py <<'PY'
import json, sqlite3
from pathlib import Path
DB="/var/lib/job-queue/jobs.sqlite3"
def db():
 c=sqlite3.connect(DB,timeout=10); c.row_factory=sqlite3.Row; return c
def initialize():
 c=db(); c.execute("CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, payload TEXT NOT NULL, status TEXT NOT NULL, result TEXT, attempts INTEGER NOT NULL DEFAULT 0)")
 for path in Path("/var/lib/accepted-jobs").glob("*.json"):
  job=json.loads(path.read_text())
  c.execute("INSERT OR IGNORE INTO jobs(id,payload,status,result,attempts) VALUES(?,?,\"queued\",NULL,0)",(job["id"],job["payload"]))
 c.commit(); c.close()
initialize()
PY
cat > /opt/job-queue/api.py <<'PY'
import json
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from common import db
class H(BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=="/health": body=b"ready"
  elif self.path.startswith("/jobs/"):
   c=db(); row=c.execute("SELECT * FROM jobs WHERE id=?",(self.path[6:],)).fetchone(); c.close()
   if not row: self.send_error(404); return
   body=json.dumps(dict(row),separators=(",",":")).encode()
  else: self.send_error(404); return
  self.send_response(200); self.send_header("Content-Type","application/json"); self.send_header("Content-Length",str(len(body))); self.end_headers(); self.wfile.write(body)
 def do_POST(self):
  if not self.path.startswith("/jobs/"): self.send_error(404); return
  payload=self.rfile.read(int(self.headers.get("Content-Length","0"))).decode(); c=db()
  try: c.execute("INSERT INTO jobs(id,payload,status,result,attempts) VALUES(?,?,\"queued\",NULL,0)",(self.path[6:],payload)); c.commit()
  except Exception: c.close(); self.send_error(409); return
  c.close(); self.send_response(202); self.end_headers()
 def log_message(self,*_): pass
ThreadingHTTPServer(("0.0.0.0",8080),H).serve_forever()
PY
cat > /opt/job-queue/worker.py <<'PY'
import time
from common import db
while True:
 c=db(); c.execute("BEGIN IMMEDIATE"); row=c.execute("SELECT id,payload FROM jobs WHERE status=\"queued\" ORDER BY rowid LIMIT 1").fetchone()
 if row: c.execute("UPDATE jobs SET status=\"completed\",result=?,attempts=attempts+1 WHERE id=? AND status=\"queued\"",("processed:"+row["payload"],row["id"])); c.commit()
 else: c.commit(); time.sleep(.1)
 c.close()
PY
install -d -m 0755 /etc/replaybook
printf "%s\n" "exec /run/current-system/sw/bin/python3 /opt/job-queue/api.py" > /etc/replaybook/job-api-start
printf "%s\n" "exec /run/current-system/sw/bin/python3 /opt/job-queue/worker.py" > /etc/replaybook/job-worker-start
chmod 0755 /opt/job-queue/*.py
systemctl reset-failed job-api.service job-worker.service
systemctl restart job-api.service job-worker.service'
jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle discovered accepted work and deployed durable queue",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
