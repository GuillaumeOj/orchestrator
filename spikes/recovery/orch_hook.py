#!/usr/bin/env python3
"""Stand-in for `orch-hook`: forwards a hook payload to the core over its Unix socket and prints the core's answer.

Env (set by the core at spawn): ORCH_AGENT_ID, ORCH_SOCK, ORCH_SPOOL (file), ORCH_HOOK_DEADLINE (s, default 540),
ORCH_HOOK_FALLBACK = deny (default, designed) | open (exit 0, no output) | exit1 (crash) | exit2 (blocking error).

When the core can't be reached (no socket, refused, EOF mid-wait, deadline hit):
- PreToolUse and PermissionRequest are answered **deny** (fail closed), unless the fallback says otherwise;
- every event is appended to ORCH_SPOOL so the core can replay it after it restarts."""
import json, os, socket, sys, time

raw = sys.stdin.read()
try:
    p = json.loads(raw)
except Exception:
    p = {"_raw": raw}
ev = p.get("hook_event_name", "?")
agent = os.environ.get("ORCH_AGENT_ID", "?")
sock_path = os.environ.get("ORCH_SOCK", "")
deadline = float(os.environ.get("ORCH_HOOK_DEADLINE", "540"))
fallback = os.environ.get("ORCH_HOOK_FALLBACK", "deny")
t0 = time.time()


def spool(why):
    path = os.environ.get("ORCH_SPOOL")
    if path:
        with open(path, "a") as f:
            f.write(json.dumps({"ts": t0, "why": why, "agent": agent, "event": ev, "payload": p}) + "\n")


def unreachable(why):
    spool(why)
    msg = f"ORCH: orchestrator unavailable ({why}); tool not run. Wait for the orchestrator, do not retry in a loop."
    if fallback == "exit1":
        sys.stderr.write(f"orch-hook: cannot reach core: {why}\n"); sys.exit(1)
    if fallback == "exit2":
        sys.stderr.write(msg + "\n"); sys.exit(2)
    if fallback == "open":
        sys.exit(0)
    if ev == "PreToolUse":
        print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                                                  "permissionDecisionReason": msg}}))
    elif ev == "PermissionRequest":
        print(json.dumps({"hookSpecificOutput": {"hookEventName": "PermissionRequest",
                                                  "decision": {"behavior": "deny", "message": msg}}}))
    sys.exit(0)


try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(1.0)
    s.connect(sock_path)
    s.sendall((json.dumps({"type": "hook", "agent": agent, "event": ev, "payload": p}) + "\n").encode())
    s.settimeout(max(1.0, deadline - (time.time() - t0)))
    buf = b""
    while not buf.endswith(b"\n"):
        d = s.recv(65536)
        if not d:
            unreachable("core closed the connection")
        buf += d
except (FileNotFoundError, ConnectionRefusedError) as e:
    unreachable(type(e).__name__)
except socket.timeout:
    unreachable("deadline")
except OSError as e:
    unreachable(f"OSError {e.errno}")

resp = json.loads(buf)
if resp.get("stdout"):
    print(resp["stdout"])
sys.exit(int(resp.get("exit", 0)))
