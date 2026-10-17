#!/usr/bin/env python3
"""Mock orch-mcp: stdio MCP server with one tool that echoes the agent identity from env."""
import json, os, sys, time
out = os.environ.get("SPIKE_OUT", "/tmp")
def send(m): sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
for line in sys.stdin:
    try: m = json.loads(line)
    except Exception: continue
    with open(os.path.join(out, "mcp.jsonl"), "a") as f: f.write(json.dumps({"ts": time.time(), "in": m, "agent": os.environ.get("ORCH_AGENT_ID")}) + "\n")
    i, meth = m.get("id"), m.get("method")
    if meth == "initialize":
        send({"jsonrpc": "2.0", "id": i, "result": {"protocolVersion": m["params"].get("protocolVersion", "2025-06-18"),
              "capabilities": {"tools": {}}, "serverInfo": {"name": "orch", "version": "0.0.1"}}})
    elif meth == "tools/list":
        send({"jsonrpc": "2.0", "id": i, "result": {"tools": [{"name": "whoami", "description": "Return the orchestrator agent identity",
              "inputSchema": {"type": "object", "properties": {}}}]}})
    elif meth == "tools/call":
        send({"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": "agent id = " + str(os.environ.get("ORCH_AGENT_ID"))}]}})
    elif i is not None:
        send({"jsonrpc": "2.0", "id": i, "result": {}})
