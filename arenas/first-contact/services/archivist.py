#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import subprocess


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/health":
            self.reply(404, "not found")
            return
        query = subprocess.run(
            ["psql", "--username", "arena", "--dbname", "empire", "--tuples-only",
             "--no-align", "--command",
             "SELECT value FROM empire_state WHERE key = 'status'"],
            capture_output=True, text=True, timeout=2, check=False)
        body = query.stdout.strip()
        self.reply(200 if query.returncode == 0 and body == "archivist:ok" else 503,
                   body if body else "database unavailable")

    def reply(self, status, body):
        encoded = body.encode()
        self.send_response(status)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, format, *args):
        pass


ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
