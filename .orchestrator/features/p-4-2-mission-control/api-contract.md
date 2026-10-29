# P4.2 — API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Extends [P4.1's contract](../p-4-1-one-now/api-contract.md): everything there stays; this file only **adds optional fields**. Back owns the Rust side, front the Svelte side; front may mock the new fields until back lands them. Field names are final.

## 1. Additions to the attention records

### 1.1 RunAttention
```jsonc
{
  "run": "run-12", "feature": "checkout", "project": "shop-api",
  "phase": "executing", "phase_since_ms": 1761720000000, "level": "needs_you",
  "members": ["lead-1", "back-1", "front-1"],     // CHANGED: lead first, then the team's node order, then agent id
  "blocker": { … },                               // unchanged
  "counts": { … },                                // unchanged
  // new
  "lead": "lead-1",                               // absent only if the run has no lead agent yet
  "started_ms": 1761700000000,                    // run.started_at_ms
  "tasks": { "total": 7, "done": 3, "in_progress": 2, "review": 1, "blocked": 1 },
                                                  // blocked = not done, with an unmet depends_on (orch_team::run::blocked_by)
                                                  // counts are disjoint except `blocked` (a subset of the not-done ones)
  "pr": { "url": "https://github.com/o/r/pull/12", "number": 12 },  // once run.pr_url is set; number parsed from the URL when it ends in /pull/<n>
  // new (D12, ux F8): for phase paused | deferred | failed only
  "held_in": "executing",                         // the step the run left: `from` of the run.state event that entered this phase,
                                                  // walking back past paused/deferred; absent if no such event
  "until_ms": 1761548400000                       // deferred only: when it resumes (the stored deferral time, else the weekly reset); absent if unknown
}
```
A change in task counts or `pr` is a record change (pushed in `now.attention.runs`). Task writes already set the `teams` / `features` topics; they must also mark attention dirty.

### 1.2 AgentAttention — queued agents
```jsonc
{
  "agent": "infra-1", "level": "queued", "status": "queued",   // or "not_started"
  "waiting_on": { "kind": "governor", "ref": "infra-1", "since_ms": 0 },
  "reason": "live cap 2/2 · 2nd in line · ≈14:20",             // unchanged (P4.1 copy)
  // new, for status queued and not_started; absent for paused and every other status
  "queue": {
    "entry": 314,              // the Governor queue entry id: what governor_act run_now / cancel take
    "position": 2,             // 1-based in the Governor's order; absent if not in the queue order (not started, no entry)
    "tier": "low",             // allocation tier: critical | high | normal | low
    "eta_ms": 1761736800000,   // absent when the Governor has no estimate
    "run_now": {               // FR-809: cost of starting it now
      "week_pp": 2.1,
      "five_hour_pp": 6,
      "binds": "week"          // week | five_hour: the window that holds it (5h guard ⇒ five_hour)
    }
  }
}
```
`queue` is present **only when the agent has a Governor queue entry**, so `entry` is always set when `queue` is (D13, ux F9); a not-started agent with no entry has no `queue` field and its row shows no Start now / Cancel (`r` resume still applies). The UI shows `run_now` in the window named by `binds` and both in the confirmation (FR-809). `runCost(queue_entry)` in `status.js` and the UI's use of `app_state.agents[].queue_entry` for Mission control go; `entry` is what `governor_act { action: "run_now" | "cancel", arg: entry }` takes (unchanged commands).

## 2. Commands
No new command. Used by Mission control:

| Command | Use |
|---|---|
| `now_snapshot` / event `now` | everything Mission control renders (P4.1 §3) |
| `agent_action { id, action }` | `interrupt`, `pause`, `resume`, `suspend`, `kill` on the target agent (unchanged) |
| `governor_act { action: run_now \| cancel, arg: entry }` | Run now / Start now / Cancel on a queued row |
| `settings_get / settings_set { key: "mission" }` | `{ "projects": ["shop-api", "blog"] }` or `{ "projects": null }` (all); UI-owned, the core stores it opaquely |

## 3. ctl / orchd
`orchd now`: run lines add the task progress and the PR: `run-12 (checkout)  executing  needs_you  3/7 tasks  PR #12  blocker: back-1 · permission: …`.
