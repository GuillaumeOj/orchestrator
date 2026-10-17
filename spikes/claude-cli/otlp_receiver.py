#!/usr/bin/env python3
"""Minimal OTLP/HTTP (json) receiver: logs every POST body to <out>/otlp.jsonl."""
import json, os, sys, time
from http.server import BaseHTTPRequestHandler, HTTPServer
out = sys.argv[1]; port = int(sys.argv[2])
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0)); body = self.rfile.read(n)
        try: b = json.loads(body)
        except Exception: b = {"_bytes": len(body), "ctype": self.headers.get("Content-Type")}
        with open(os.path.join(out, "otlp.jsonl"), "a") as f:
            f.write(json.dumps({"ts": time.time(), "path": self.path, "body": b}) + "\n")
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.end_headers(); self.wfile.write(b"{}")
    def log_message(self, *a): pass
HTTPServer(("127.0.0.1", port), H).serve_forever()
