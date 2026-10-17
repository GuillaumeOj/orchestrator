#!/usr/bin/env python3
"""Spike hook: logs every hook payload; applies a policy for PreToolUse/PostToolUse/Stop.
Env: SPIKE_OUT (dir), SPIKE_POLICY (json file, re-read on every call)."""
import json, os, sys, time
out = os.environ.get("SPIKE_OUT", "/tmp/spike")
raw = sys.stdin.read()
try:
    p = json.loads(raw)
except Exception:
    p = {"_raw": raw}
ev = p.get("hook_event_name", "?")
rec = {"ts": time.time(), "event": ev, "argv": sys.argv[1:], "env_agent": os.environ.get("ORCH_AGENT_ID"), "payload": p}
with open(os.path.join(out, "hooks.jsonl"), "a") as f:
    f.write(json.dumps(rec) + "\n")
pol = {}
try:
    pol = json.load(open(os.environ.get("SPIKE_POLICY", os.path.join(out, "policy.json"))))
except Exception:
    pass
resp = None
if ev == "PreToolUse":
    cmd = json.dumps(p.get("tool_input", {}))
    for rule in pol.get("pre", []):
        if rule.get("match", "") in cmd:
            if rule.get("sleep"):
                time.sleep(rule["sleep"])
            resp = {"hookSpecificOutput": {"hookEventName": "PreToolUse",
                    "permissionDecision": rule["decision"],
                    "permissionDecisionReason": rule.get("reason", "spike")}}
            break
elif ev == "PostToolUse" and pol.get("post_context"):
    resp = {"hookSpecificOutput": {"hookEventName": "PostToolUse",
            "additionalContext": pol["post_context"]}}
elif ev == "Stop" and pol.get("stop_continue"):
    flag = os.path.join(out, "stop_continued")
    if not os.path.exists(flag) and not p.get("stop_hook_active"):
        open(flag, "w").write("1")
        resp = {"decision": "block", "reason": pol["stop_continue"]}
elif ev == "PermissionRequest" and pol.get("perm_request"):
    pr = pol["perm_request"]
    if pr.get("sleep"):
        time.sleep(pr["sleep"])
    dec = {"behavior": pr["behavior"]}
    if pr["behavior"] == "deny":
        dec["message"] = pr.get("message", "denied by orchestrator")
    resp = {"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": dec}}
if resp is not None:
    with open(os.path.join(out, "hook_responses.jsonl"), "a") as f:
        f.write(json.dumps({"ts": time.time(), "event": ev, "resp": resp}) + "\n")
    print(json.dumps(resp))
sys.exit(0)
