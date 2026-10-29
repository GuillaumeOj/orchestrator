# Improve the end of a feature · API contract (core ↔ UI)

> Companion of [spec.md](spec.md). Back owns the Rust side, front the Svelte side; front codes against this file and can mock it until back lands.

## 1. Run state

`run.status` gains `"pr_open"` (between `"pr_gate"` and `"done"`). `resume_status` may be `"pr_open"` on a paused / deferred run. `"done"` now means the PR was merged.

## 2. Run JSON (`feature_view` → `run`, `runs_json`, `orchd`)

`run` keeps its fields (`id, team_id, team_scope, team_version, feature_id, status, resume_status, pr_url, started_at_ms, ended_at_ms`) and gains:

```json
"pr_number": 12,                 // null before a PR is opened
"pr": {                          // null before the first check
  "state": "open",               // "open" | "merged" | "closed"
  "head_sha": "8732db1…",
  "base": "main",
  "ci": {
    "overall": "failing",        // "passing" | "failing" | "pending" | "none"
    "passing": 4, "failing": 1, "pending": 0, "skipped": 1,
    "failing_checks": [ { "name": "ci / test", "url": "https://github.com/…/runs/…" } ]  // at most 10
  },
  "position": {
    "status": "behind",          // "up_to_date" | "behind" | "conflicts" | "unknown"
    "behind": 3,                 // commits of origin/<base> not in the PR head
    "ahead": 14                  // commits of the PR head not in origin/<base>
  },
  "unpushed": 2,                 // local integration commits not in head_sha
  "checked_at_ms": 1793260000000,
  "error": null                  // a string when the last check failed: the rest is the last good status (stale)
}
```

## 3. Commands

| Command | Args | Returns | Notes |
|---|---|---|---|
| `pr_refresh` | `{ feature: string }` | the updated `run` JSON | Checks the feature's latest run's PR now. Error when the run has no `pr_number` or the app is offline. |

Same as ctl request `PrRefresh { feature }` → `orchd pr refresh <feature>`.

## 4. Timeline events (`run_event`, shown via `timelineText`)

| kind | detail |
|---|---|
| `pr.opened` | `{ number, url }` |
| `pr.status` | `{ number, ci?: {from, to, failing_checks?}, position?: {from, to, behind} }` (only the parts that changed) |
| `pr.pushed` | `{ number, commits }` |
| `pr.merged` | `{ number }` |
| `pr.closed` | `{ number }` |

`run.state` events keep their shape (`from`, `to`, `why`); new `why`s: `"pull request opened"` (→ `pr_open`), `"pull request merged"` (→ `done`), `"pull request closed"` (→ `integrating`).

## 5. Inbox

- New gate action `push_pr_update` (kind `gate`): `payload { action: "push_pr_update", run, branch, pr_number, url, commits: [{sha, subject}], worktree }`. Actions as other gates: approve / reject (with a message). The card shows the PR number and the commit subjects.
- Info items `pr_merged` / `pr_closed` (level 3, no action but dismiss): `payload { run, feature, pr_number, url }`.

## 6. Orchestrator MCP (agents)

| Tool | Who | Args | Effect |
|---|---|---|---|
| `sync_base` | lead | `{}` | Fetches `origin/<base>`, merges it into the integration worktree. Returns `Merged {sha}` / `UpToDate` / `Conflict {files}` (fix, then `commit`). |
| `request_pr` | lead | `{title, body, draft}` (ignored in `pr_open`) | In `pr_open`: a `push_pr_update` gate instead of a new PR. |
