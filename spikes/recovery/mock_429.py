#!/usr/bin/env python3
"""Local stand-in for the API that answers every /v1/messages call with a plan rate-limit 429.
Logs only method, path and status: request headers (which carry the OAuth token) and bodies are never read or stored."""
import http.server, json, sys, time
LOG = open(sys.argv[2], "a")
RESET = int(time.time()) + 3600
class H(http.server.BaseHTTPRequestHandler):
    def _answer(self):
        n = int(self.headers.get("content-length") or 0)
        if n: self.rfile.read(n)                      # drained, not kept
        if self.path.startswith("/v1/messages"):
            body = json.dumps({"type": "error", "error": {"type": "rate_limit_error",
                               "message": "This request would exceed your account's rate limit. Please try again later."}}).encode()
            self.send_response(429)
            for k, v in {"content-type": "application/json", "x-should-retry": "false",
                         "anthropic-ratelimit-unified-status": "rejected",
                         "anthropic-ratelimit-unified-5h-status": "rejected",
                         "anthropic-ratelimit-unified-5h-utilization": "1.0",
                         "anthropic-ratelimit-unified-5h-reset": str(RESET),
                         "anthropic-ratelimit-unified-reset": str(RESET),
                         "anthropic-ratelimit-unified-representative-claim": "five_hour"}.items():
                self.send_header(k, v)
            code = 429
        else:
            body = b"{}"; self.send_response(404); self.send_header("content-type", "application/json"); code = 404
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
        LOG.write(json.dumps({"ts": time.time(), "method": self.command, "path": self.path.split("?")[0], "status": code}) + "\n"); LOG.flush()
    do_POST = do_GET = do_HEAD = _answer
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
