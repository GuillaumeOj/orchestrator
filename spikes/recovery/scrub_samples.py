#!/usr/bin/env python3
"""Pull representative payloads out of the spike DBs and scrub them. usage: scrub_samples.py SCRATCH OUTDIR"""
import glob, json, os, re, sqlite3, sys
scratch, out = sys.argv[1], sys.argv[2]
home = os.path.expanduser("~")
slug = re.sub(r"[/.]", "-", scratch)
def scrub(s):
    s = s.replace(scratch, "<scratch>").replace(slug, "<scratch-slug>").replace(home, "~")
    s = re.sub(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[a-z]{2,}", "<email>", s)
    s = re.sub(r"\"signature\": ?\"[^\"]+\"", '"signature": "<redacted>"', s)
    return re.sub(r"(sk-ant-|oat01-)[A-Za-z0-9_-]+", "<token>", s)
def q(db, sql, *a):
    c = sqlite3.connect(db); return c.execute(sql, a).fetchall()
R = os.path.join(scratch, "r/app/state.db")
pick = {}
def first(name, agent, kind, where=""):
    rows = q(R, f"select payload from events where agent_id=? and kind=? {where} order by ts", agent, kind)
    if rows: pick[name] = json.loads(rows[0][0])
first("SubagentStart (Agent tool, async)", "b5-sub", "SubagentStart")
first("PostToolUse (Agent tool, async_launched)", "b5-sub", "PostToolUse")
first("Stop (with a running background subagent)", "b5-sub", "Stop")
first("SubagentStop (general-purpose)", "b5-sub", "SubagentStop")
rows = q(R, "select payload from events where agent_id='b5-sub' and kind='UserPromptSubmit' order by ts")
pick["UserPromptSubmit (task-notification turn started by the CLI)"] = json.loads(rows[1][0])
rows = q(R, "select payload from events where agent_id='b5-sub' and kind='SubagentStop' order by ts")
pick["SubagentStop (internal, agent_type empty, no SubagentStart)"] = json.loads(rows[1][0])
first("PreCompact (manual)", "b5-sub", "PreCompact")
first("SessionStart (source compact)", "b5-sub", "SessionStart", "and payload like '%compact%'")
first("PostCompact (manual)", "b5-sub", "PostCompact")
first("StopFailure (model_not_found)", "b6-model", "StopFailure")
first("StopFailure (rate_limit, local 429 mock)", "b6-429", "StopFailure")
rows = q(R, "select payload from events where agent_id='b4-hold' and kind='UserPromptSubmit' order by ts")
pick["UserPromptSubmit (typed while a PreToolUse hook was held: queued, same prompt_id)"] = json.loads(rows[1][0])
first("Stop (background shell running)", "e-bg", "Stop")
first("SessionEnd (after PTY hangup; reason other)", "f-gov", "spooled:SessionEnd")
for k, v in pick.items():
    for f in ("last_assistant_message", "compact_summary"):
        if isinstance(v.get(f), str) and len(v[f]) > 300: v[f] = v[f][:300] + "…"
    if isinstance(v.get("tool_response"), dict) and "outputFile" in v["tool_response"]: pass
json.dump(pick, open(os.path.join(out, "hook-payloads.json"), "w"), indent=1)
open(os.path.join(out, "hook-payloads.json"), "w").write(scrub(json.dumps(pick, indent=1)))
# statusline after the 429 mock
st = [json.loads(json.loads(l)["raw"]) for l in open(os.path.join(scratch, "r/app/agents/b6-429/status.jsonl")) if 'rate_limits' in l and 'null' not in l.split('rate_limits')[1][:8]]
sl = {"after 429 (mock)": {k: st[-1].get(k) for k in ("rate_limits", "cost", "version")}}
for a in ("b7-x", "b7-y", "b7-z"):
    s = [json.loads(json.loads(l)["raw"]) for l in open(os.path.join(scratch, f"r/app/agents/{a}/status.jsonl")) if 'rate_limits' in l and 'null' not in l.split('rate_limits')[1][:8]]
    sl[f"three agents: {a}"] = s[-1]["rate_limits"]
open(os.path.join(out, "statusline.json"), "w").write(scrub(json.dumps(sl, indent=1)))
# orch-hook spool (core down) + one reconcile report
sp = []
for f in sorted(glob.glob(os.path.join(scratch, "r/app/agents/*/spool.jsonl*"))):
    for l in open(f):
        e = json.loads(l); sp.append({"agent": e["agent"], "event": e["event"], "why": e["why"]})
open(os.path.join(out, "spool.json"), "w").write(scrub(json.dumps(sp, indent=1)))
rep = json.load(open(sorted(glob.glob(os.path.join(scratch, "r/app/reconcile-*.json")))[-1]))
for a in rep["agents"]:
    if a["transcript"].get("found"):
        a["transcript"].pop("path", None)
open(os.path.join(out, "reconcile-round3.json"), "w").write(scrub(json.dumps(rep, indent=1, default=str)))
print("ok", list(pick))
