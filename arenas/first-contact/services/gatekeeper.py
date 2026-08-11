#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROUTE = Path("/var/lib/gatekeeper/route")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            route = ROUTE.read_text().strip() if ROUTE.exists() else "missing"
            self.reply(200 if route == "checkout" else 502,
                       "gatekeeper:ok" if route == "checkout" else "bad route")
        elif self.path == "/routes/checkout":
            self.reply(200, ROUTE.read_text().strip())
        else:
            self.reply(404, "not found")

    def do_PUT(self):
        if self.path != "/routes/checkout":
            self.reply(404, "not found")
            return
        length = int(self.headers.get("content-length", "0"))
        ROUTE.write_bytes(self.rfile.read(length))
        self.reply(202, "route accepted")

    def reply(self, status, body):
        encoded = body.encode()
        self.send_response(status)
        self.send_header("content-type", "text/plain")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, format, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", 9000), Handler).serve_forever()
