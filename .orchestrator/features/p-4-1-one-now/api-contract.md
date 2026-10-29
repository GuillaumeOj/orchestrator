# P4.1 — API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Back owns the Rust side, front the Svelte side. Back lands §1–3 first (task B1), §2/§4 next (B2); front codes against this file and may mock the `now` event until then. Field names are final; adding optional fields later is allowed, renaming is not.

## 1. Attention records

```jsonc
// AgentAttention
{
  "agent": "front-1",
  "project": "shop-api",            // seat project, else the session's project
  "run": "run-12",                  // absent for solo sessions
  "node": "front",                  // absent for solo sessions
  "task": 4,                        // the task in progress, if any
  "level": "needs_you",             // needs_you | running | waiting | queued | idle | done | error
  "status": "waiting_for_you",      // working | waiting_for_you | waiting_gate | queued | not_started | paused
                                    // | blocked_by_limit | idle | suspended | error | done
  "waiting_on": {                   // absent for running / idle / done
    "kind": "user",                 // user | agent | task | governor | limit | startup | external
    "ref": "inbox-42",              // decision id (user) · agent id (agent, governor) · "T-2" (task) · window (limit) · tool (external)
    "since_ms": 1761730000000
  },
  "reason": "permission: Bash pnpm add zod",   // never repeats the status label, no duration (UI appends since); clock times allowed
  "activity": "Edit src/feed.ts",   // running only
  "live": true,
  "since_ms": 1761730000000         // when this level + wait began
}
```
`level == "needs_you"` ⇔ `waiting_on.kind == "user"`. `status` replaces the UI's `displayState()`. Rules and precedence: spec §3.1.

```jsonc
// RunAttention
{
  "run": "run-12", "feature": "checkout", "project": "shop-api",
  "phase": "executing",             // RunState: draft | spec | spec_gate | planning | executing | integrating
                                    // | pr_gate | done | paused | deferred | failed
  "phase_since_ms": 1761720000000,
  "level": "needs_you",
  "members": ["lead-1", "back-1", "front-1"],
  "blocker": { "agent": "front-1", "waiting_on": { "kind": "user", "ref": "inbox-42", "since_ms": 0 },
               "reason": "permission: Bash pnpm add zod" },   // absent when nothing external
  "counts": { /* Counts, this run only */ }
}
```

```jsonc
// Counts — global, per project ("projects": {id: Counts}), per run (in RunAttention)
{ "needs_you": 2, "decisions": 2, "running": 2, "waiting": 1, "queued": 2, "idle": 0, "error": 0,
  "live": 4, "live_cap": 3, "done_today": 5 }
```
`decisions` = open decisions of levels 1–3 waiting on the user (the title badge). `queued` = agents at level `queued` only (no double count: an agent with a Governor ask is at needs_you). `live` excludes archived agents. `live_cap` is global only (absent per project/run).

## 2. Decisions

### 2.1 Shape (replaces the Inbox item)
```jsonc
{
  "id": "inbox-42",                 // stable; synthesized ones keep their prefixes (trust-, wait-, error-, ask-)
  "kind": "permission",             // permission | question | gate | startup | alert | budget | ask | trust | wait | error | notion
  "gate": "spec",                   // gate only: spec | pr | restart
  "level": 1,                       // 1 blocking · 2 errors · 3 budget  (level 4 is outcomes, §2.4)
  "subject": { "agent": "front-1", "role": "Frontend", "node": "front", "project": "shop-api",
               "feature": "checkout", "run": "run-12", "task": 4 },
  "title": "Bash · pnpm add zod",
  "summary": "Add zod",             // one line, may be empty
  "context": [ /* blocks, §2.2, in display order */ ],
  "actions": [ { "id": "approve", "label": "Allow once", "key": "a" },
               { "id": "reject",  "label": "Deny",  "key": "r" },
               { "id": "reply",   "label": "Deny with message", "key": "enter" },
               { "id": "open",    "label": "Open pane", "key": "o" } ],
  "lifecycle": {
    "state": "open",                // open | parked | decided | sent | effect | failed | stale | dismissed | interrupted
    "decided_by": "user",           // user | pane | rule:<id> | expired | core | policy | overwatch — from decided on
    "decided_at_ms": 0, "sent_at_ms": 0,
    "effect": "resumed",            // allowed | denied | delivered | resumed | queued | pr_opened | restarted | dismissed
                                    // | applied | skipped | deleted
    "effect_detail": "≈14:20",      // ETA for queued, URL for pr_opened
    "effect_at_ms": 0,
    "error": "push rejected: …",    // failed only
    "stale_why": "the agent moved on",
    "version": 3                    // +1 on every lifecycle write (also in inbox_act's answer): keep the highest: ignore an upsert or answer with a lower version; an equal version replaces the card's fields without a second toast (the core re-pushes display changes at the same version)
  },
  "thread": "run-12",               // run id, else agent id
  "thread_prev": "inbox-37",        // previous decision in the thread, if any
  "waits_on": "user",               // user | core (core: being delivered; not counted in the badge)
  "created_ms": 0,
  "payload": { /* raw, kind-specific, kept for compatibility; the UI renders `context` */ }
}
```
`actions[].id` is what `inbox_act` takes (§4). Keys follow FR-707.

### 2.2 Context blocks
```jsonc
{ "type": "tool_call",    "tool": "Bash", "input": { … }, "cwd": "/…/front-1" }
{ "type": "markdown",     "text": "…", "source": "spec.md" }
{ "type": "docs",         "files": [ { "path": ".orchestrator/features/checkout/spec.md", "lines": 142 } ] }
{ "type": "diff_summary", "files": [ { "path": "src/a.rs", "added": 40, "removed": 3 } ], "added": 142, "removed": 9 }
                          // patch via `decision_patch { id, path? }`
{ "type": "options",      "questions": [ { "question": "…", "header": "Auth", "multi": false,
                                           "options": [ { "label": "…", "description": "…" } ] } ] }
{ "type": "last_message", "text": "…", "at_ms": 0 }
{ "type": "pty_tail",     "text": "…ANSI-stripped…" }
{ "type": "cost",         "week_pp": 2.1, "five_hour_pp": 6, "eta_ms": 0 }
{ "type": "countdown",    "default": "approve", "deadline_ms": 0 }
{ "type": "error",        "text": "…", "detail": "…" }
```

### 2.3 Kind → blocks
| Kind | Blocks (in order) |
|---|---|
| permission | tool_call · (parked: none extra; P4.0 `payload.restarted` kept) |
| question (ask_user, AskUserQuestion) | markdown (the question) · options · last_message? |
| gate spec | markdown (spec summary) · docs |
| gate pr | markdown (title + body) · diff_summary |
| gate restart | markdown (detail) |
| startup | error (why, exit status) · pty_tail |
| ask (Governor) | markdown (what waits) · cost · countdown |
| budget | markdown (reason) · cost · countdown? |
| trust | pty_tail |
| wait (prompt only in its pane) | last_message? · pty_tail |
| error / alert | error · pty_tail? (spawn, API, watchdog) · markdown (worktree, battery, MCP auth) |
| notion | markdown (comment) · `source` = page URL |

### 2.4 Outcomes (level 4)
```jsonc
{ "id": "ev-9812", "kind": "task_done",  "at_ms": 0, "project": "shop-api", "feature": "checkout", "run": "run-12",
  "agent": "back-1", "text": "T-4 done · checkout API", "task": 4 }
// every outcome has core-written `text` (patterns: docs/design/p4-design-notes.md §5) and `feature` when it has a run
{ "kind": "merge_done", "branch": "orch/checkout/back-1", "into": "orch/checkout/integration", … }
{ "kind": "pr_opened",  "url": "https://github.com/…/pull/12", … }
{ "kind": "run_phase",  "from": "planning", "to": "executing", … }
{ "kind": "run_done",   … }
{ "kind": "turn_done",  "text": "Fixed the feed pagination and added a test", … }   // solo sessions only
{ "kind": "report_ready", "report": "2026-W44", … }      // was the weekly-report info item
{ "kind": "sync_conflict", "task": 4, "backend": "linear", … }  // was the task-sync conflict info item
```
`seen: bool` per outcome (today's watermark).

## 3. Push channel

### 3.1 Event `now` (Tauri), the only state push
```jsonc
{
  "rev": 1834,                      // +1 per event; gap ⇒ call now_snapshot
  "attention": { "agents": [ AgentAttention… ], "runs": [ RunAttention… ],
                 "removed": { "agents": ["x"], "runs": [] } },     // changed records only
  "counts": { "global": Counts, "projects": { "shop-api": Counts } },  // present when any changed
  "decisions": { "upsert": [ Decision… ], "closed": ["inbox-40"] },  // closed = left the open list (its last upsert carries the final lifecycle)
  "outcomes": [ Outcome… ],         // new ones
  "topics": [ "queue", "feature:checkout", "worktree:/…/back-1" ]
}
```
Emitted at most every 200 ms (pump), only when something changed; the 30 s tick may emit reason/time updates. Topics: `agents, queue, usage, inbox, features, feature:<id>, board:<project>, planner, teams, project:<id>, worktree:<path>, report, integrations, memory, power, banners, setup`.

### 3.2 Commands
| Command | Returns / effect |
|---|---|
| `now_snapshot` | `{ rev, agents, runs, counts, decisions (open + closed in the last 10 min), outcomes (today) }` |
| `app_state` | Unchanged keys except: `inbox` is removed (decisions come from `now`), `agents[]` loses nothing (raw data stays for details), and it is **not** polled. |
| `inbox_act { item, action, message? }` | Unchanged signature; returns `{ ok, lifecycle }` (the state after the call: usually `decided` or `sent`). The final effect arrives in `now.decisions`. |
| `decision_patch { id, path? }` | The PR gate's patch (whole, or one file). |
| `digest_seen { up_to }` | Marks today's outcomes up to that id as seen. |
| `inbox_outcome` event | Still emitted in P4.1 (same payload as P4.0), deprecated; front moves to `now.decisions`. Removed in P4.3. |

## 4. Actions by kind
| Kind | Action ids |
|---|---|
| permission | approve, reject, reply, always (when offered), in_pane (open hook only), open |
| question | answer (`message` = text or the chosen option labels joined by `\n`), reply, reject (team question: dismiss), open |
| gate spec / pr / restart | approve, reject, reply, open |
| startup | restart_fresh, dismiss, delete (after a failed fresh restart), open |
| ask | approve (run now), reject (keep queued), open |
| budget | approve, reject |
| trust, wait | open |
| error / alert | open, dismiss, resume (agent error), restart_fresh (spawn/startup errors), reconnect + open_settings (MCP needs auth) |
| notion | reply, dismiss, open (the page) |

## 5. ctl / orchd (no UI)
- `CtlRequest::Now` → the `now_snapshot` body. `orchd now [--json] [--run <id>]`: one line per agent `agent  level/status  reason  since`, then one per run `run  phase  level  blocker`.
