#!/usr/bin/env bash
set -euo pipefail
source "$AOE_CREDENTIAL_FILE"
askpass="$(mktemp)"; trap 'rm -f "$askpass"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" %q\n' "$AOE_SSH_PASSWORD" >"$askpass"; chmod 700 "$askpass"
opts=(-p "$AOE_SSH_PORT" -o BatchMode=no -o PreferredAuthentications=password -o PubkeyAuthentication=no -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)
remote=(env SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 ssh "${opts[@]}" "root@${AOE_TERRITORY_HOST}")
"${remote[@]}" 'install -d -m 0755 /opt/job-queue
cat > /opt/job-queue/common.py <<'PY'
import sqlite3
DB="/var/lib/job-queue/jobs.sqlite3"
def db():
 c=sqlite3.connect(DB,timeout=10); c.row_factory=sqlite3.Row; return c
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
chmod 0755 /opt/job-queue/*.py
systemctl reset-failed queue-api.service queue-worker.service
systemctl restart queue-api.service queue-worker.service'
jq -n --arg agent "$AOE_AGENT_ID" --arg territory "$AOE_TERRITORY_ID" '{schema_version:1,agent:$agent,territory:$territory,status:"completed",summary:"oracle deployed durable exactly-once queue",usage:{resource_units:1},transcript:null}' >"$AOE_RESULT_FILE"
