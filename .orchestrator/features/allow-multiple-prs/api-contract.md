# Allow multiple PRs · API contract (core ↔ UI, MCP)

> Companion of [spec.md](spec.md). Back owns the Rust side, front the Svelte side; front codes against this file and can mock it until back lands. Fields not named here keep their shape.

## 1. Feature JSON (`feature_view` → `feature`, `features_json`, `orchd`)

```json
"issues": [                                   // [] when none; order = the user's
  { "ref": "#12", "key": "12", "title": "Crash on empty brief", "url": "https://github.com/o/r/issues/12" },
  { "ref": "ENG-7", "key": "eng-7", "title": null, "url": null }   // title / url: best effort from the intake cache
],
"pr_mode": "single"                           // "single" | "per_issue"
```

## 2. PRs of a run: `run.prs_view` (in every `run` JSON: `feature_view`, `runs_json`, mission rows)

One list for both modes, built by the core. The UI reads only this for PR surfaces; `pr_url` / `pr_number` / `pr` stay on `run` unchanged (single mode) for older code.

```json
"prs_view": [
  {
    "issue": "#12",                 // null in single mode (the one entry is the feature's PR)
    "branch": "orch/<slug>/issue/12",   // the integration branch in single mode
    "state": "open",                // "none" (no PR yet) | "pending" (a push_pr gate waits) | "open" | "merged" | "closed"
    "number": 34,                   // null for none / pending
    "url": "https://github.com/o/r/pull/34",
    "pr": { … },                    // PrStatus exactly as improve-end-of-feature api-contract §2; null before the first check
    "gate": 812                     // the pending push_pr or push_pr_update gate id, else null
  }
]
```

- **single mode**: `[]` before any PR or gate; then one entry (`issue: null`).
- **per_issue mode**: one entry **per feature issue**, in `feature.issues` order, `state: "none"` until requested. A closed PR stays `closed` until a new one replaces it.
- `pr.unpushed` (per_issue): integration commits attributed to the issue or shared since its last build. `pr.rewritten`: the fork point moved (after `rebase_base`): the next push is a force push with lease.
- Run summary for compact places (mission row, run inspector header) — computed by the UI from `prs_view` (pure helper, ux-brief §4).

## 3. Commands

| Command | Args | Returns | Errors (`{code, message}` JSON text) |
|---|---|---|---|
| `launch_team` (existing) | `LaunchSpec` gains `issues?: string[]` (refs as typed), `pr_mode?: "single" \| "per_issue"` | as today | `bad_ref {ref}` (not parsable, or a GitHub repo other than the project's), `too_many` (> 10), `per_issue_needs_two` |
| `feature_set_prs` (new) | `{ feature: id \| slug, issues?: string[], pr_mode?: "single" \| "per_issue" }` (the full new list when given) | the `feature` JSON | `bad_ref`, `too_many`, `per_issue_needs_two`, `issue_has_pr {ref}` (removing an issue with a pending, open or merged PR), `pr_busy` (mode change while a PR is pending or open), `finished` (run done) |
| `issue_suggest` (new) | `{ project, text }` | `[{ref, key, title, url}]` ≤ 8 | Matches the project's intake rows by ref or title (no network). Empty when nothing matches. |
| `intake_prefill` (existing) | | the form gains `issues: [ref]` (the item's, when GitHub or Linear) and `pr_mode: "single"` | |
| `pr_refresh` (existing) | | checks every open PR of the run | |

Ctl / `orchd`: `launch … --issue <ref> (repeat) --pr-mode <m>`, `feature prs <feature> [--issue <ref>]… [--pr-mode <m>]`.

## 4. Tasks

`task` JSON gains `"issue": "#12"` (or `null`). `task_update` from the UI does not set it (lead only).

## 5. Timeline events

Existing PR events gain `issue` (`null` in single mode): `pr.opened {number, url, issue}`, `pr.status {…, issue}`, `pr.pushed {number, commits, force, issue}`, `pr.merged {number, issue}`, `pr.closed {number, issue}`.
New:

| kind | detail |
|---|---|
| `feature.prs` | `{ issues: [ref], pr_mode, by: "user" }` — the user changed them |
| `pr.build` | `{ issue, ok, commits, shared, skipped, conflict?: {task?, commit, files} }` — a build for `request_pr` |

`run.state` new `why`s: `"every issue PR merged"` (→ `done`), `"no PR open"` (→ `integrating`, the last open PR closed, or merged with issues left).

## 6. Inbox

- `push_pr` and `push_pr_update` payloads gain `issue` (null in single mode) and, for `push_pr`, `fork_sha`, `integration_head`. In per_issue mode `branch` is the issue branch.
- Info items `pr_merged` / `pr_closed` gain `issue`. The "Feature <slug> merged" item comes once, when the run is `done`.

## 7. Orchestrator MCP (agents)

| Tool | Who | Change |
|---|---|---|
| `request_pr` | lead | `{ issue?, title, body, draft? }`. per_issue: `issue` required; builds the issue branch, then a `push_pr` gate (no PR yet) or `push_pr_update` gate (PR open). Refused: build conflict (`task`, commit, files in the text), up to date, a gate of that issue pending. single: `issue` refused; as today, plus `Closes` lines. |
| `create_task` / `update_task` | lead | `issue?: string` — a feature issue ref; else refused. |
| `merge` | lead | per_issue: `task` required. Always adds trailer `Orch-Task: <id>` when `task` is given. Refuses `orch/<slug>/issue/…`. |
| `commit` | lead | `issue?: string` → trailer `Orch-Issue: <ref>`. |
| `team_status` | all | gains `issues`, `pr_mode`, and per issue `{tasks: [handle], pr: {number, state}}`. |
