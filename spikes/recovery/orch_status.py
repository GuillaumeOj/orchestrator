#!/usr/bin/env python3
"""Stand-in for `orch-status`: logs the statusline JSON to the agent folder, forwards it to the core (best effort)."""
import json, os, socket, sys, time

raw = sys.stdin.read()
d = os.environ.get("ORCH_AGENT_DIR")
if d:
    with open(os.path.join(d, "status.jsonl"), "a") as f:
        f.write(json.dumps({"ts": time.time(), "raw": raw}) + "\n")
try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(0.5)
    s.connect(os.environ.get("ORCH_SOCK", ""))
    s.sendall((json.dumps({"type": "status", "agent": os.environ.get("ORCH_AGENT_ID"), "raw": raw}) + "\n").encode())
except OSError:
    pass
try:
    rl = json.loads(raw).get("rate_limits") or {}
    print(f"ORCH 5h={rl.get('five_hour', {}).get('used_percentage')} 7d={rl.get('seven_day', {}).get('used_percentage')}")
except Exception:
    print("ORCH ?")
