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
    "overall": "error",          // dot color: "ok" (green) | "error" (red) | "running" (yellow) | "waiting" (grey) | "none"
    "done": 4,                   // finished jobs, whatever the outcome (ok, failed, cancelled, skipped)
    "total": 6,                  // every job on the PR head, before the 100 cap
    "jobs": [                    // every job (max 100): errors first, then running, waiting, the rest; by name
      { "name": "test", "workflow": "ci", "state": "failure", "url": "https://github.com/…/runs/…" },
      { "name": "e2e", "workflow": "ci", "state": "in_progress", "url": "…" },
      { "name": "deploy-preview", "workflow": null, "state": "queued", "url": null }
    ]
    // job.state: success | neutral | skipped | failure | cancelled | timed_out | action_required |
    //            startup_failure | stale | error (finished) · in_progress | queued | waiting |
    //            requested | pending | expected (not finished)
  },
  "position": {
    "status": "behind",          // "up_to_date" | "behind" | "conflicts" | "unknown"
    "behind": 3,                 // commits of origin/<base> not in the PR head
    "ahead": 14,                 // commits of the PR head not in origin/<base>
    "base_sha": "f00ba12…"       // the origin/<base> head behind / ahead were counted against
  },
  "pushed_sha": "8732db1…",      // the PR head the app last pushed: the force-with-lease lease
  "unpushed": 2,                 // local integration commits not in head_sha
  "rewritten": false,            // the local branch no longer contains pushed_sha (a rebase): the next push force-pushes (with lease)
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
| `pr.status` | `{ number, base, ci?: {from, to, failed_jobs?: [{name, workflow}]}, position?: {from, to, behind} }` (only the parts that changed; `ci` only on changes to or from `error` / `ok`) |
| `pr.pushed` | `{ number, commits, force }` |
| `git.rebase` | `{ from, to, onto, base, base_commits, branches_moved: [branch] }` (`onto` = `origin/<base>`; `base_commits`: commits of `onto` newly under the feature) |
| `pr.merged` | `{ number }` |
| `pr.closed` | `{ number }` |

`run.state` events keep their shape (`from`, `to`, `why`); new `why`s: `"pull request opened"` (→ `pr_open`), `"pull request merged"` (→ `done`), `"pull request closed"` (→ `integrating`).

## 5. Inbox

- New gate action `push_pr_update` (kind `gate`): `payload { action: "push_pr_update", run, branch, pr_number, url, commits: [{sha, subject}], force, lease_sha, foreign_replaced, head, base, worktree }` (`lease_sha` = the PR head the app last saw: `pr.pushed_sha`, or `pr.head_sha` when someone else pushed only commits already in `<base>` (GitHub's "Update branch"); `foreign_replaced` = how many of those commits pushed outside the app the push replaces, 0 otherwise: the card then says "replaces N commits pushed outside the app (already in <base>)"; `head` = the commit pushed). When the PR holds commits pushed outside the app that are neither in the branch nor in `<base>`, no gate is created: `request_pr` is refused and the lead told to ask the user. The run stays `pr_open` on approve, reject or a failed push. Actions as other gates: approve / reject (with a message). The card shows the PR number, the commit subjects, and when `force` a clear "rewrites the PR's history (rebased onto <base>, force-with-lease)" line.
- Info items `pr_merged` / `pr_closed` (level 4, no action but dismiss): `payload { run, feature, pr_number, url }`.

## 6. Orchestrator MCP (agents)

| Tool | Who | Args | Effect |
|---|---|---|---|
| `rebase_base` | lead | `{ abort?: bool }` | Fetches `origin/<base>` and rebases the integration branch onto it. Refused while a task holds unmerged work or a PR / update gate is pending. Returns `Rebased {from, to, branches_moved}` / `UpToDate` / `Conflict {files, step, of}` (fix, then `commit`) / `Aborted`. |
| `commit` | all | as today | While a rebase is in progress in the worktree: stages and runs `git rebase --continue`; returns the next rebase outcome. |
| `request_pr` | lead | `{title, body, draft}` (ignored in `pr_open`) | In `pr_open`: a `push_pr_update` gate instead of a new PR. |
