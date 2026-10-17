#!/usr/bin/env python3
import json, os, sys, time
out = os.environ.get("SPIKE_OUT", "/tmp/spike")
raw = sys.stdin.read()
with open(os.path.join(out, "statusline.jsonl"), "a") as f:
    f.write(json.dumps({"ts": time.time(), "raw": raw}) + "\n")
try:
    d = json.loads(raw); rl = d.get("rate_limits") or {}
    print(f"SPIKE 5h={rl.get('five_hour',{}).get('used_percentage')} 7d={rl.get('seven_day',{}).get('used_percentage')}")
except Exception:
    print("SPIKE ?")
