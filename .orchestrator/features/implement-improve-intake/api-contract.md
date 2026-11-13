# Implement Improve Intake · API contract (core ↔ UI), consolidated

> Companion of [spec.md](spec.md). This file is **binding** for back and front. It is [improve-intake/api-contract.md](../improve-intake/api-contract.md) with findings F1–F40 of [the design notes §14](../../../docs/design/improve-intake-design-notes.md) folded in (where they differ, this file follows the finding). Copy, states and keys are in the design notes (**notes §n**), not here. Field names are snake_case in JSON; Tauri command arguments are passed as the existing commands do (`invoke('cmd', { project, settings })`). A shape change is agreed with lead first (and written here).

## 1. Tracker settings (B1 · F1)

```jsonc
Tracker =
  { "kind": "local",  "backlog_glob": ".orchestrator/backlog/*.md" | null }
| { "kind": "github", "repo": "owner/name" | null /* null = project's remote */, "project_board": { "owner": "acme", "number": 4 } | null }
| { "kind": "linear", "teams": ["ENG"], "projects": ["<project id>"] | null /* null = every project of the teams */ }

TrackerSettings = {
  "tracker": Tracker,
  "ready": ReadyFilter,
  "write_back": {
    "enabled": true,                 // default true for github / linear (improve-intake spec Q1); ignored for local
    "assign_me": false,
    "running_label": "orch:running", // github without board
    "board_status": "In progress" | null,   // github with board: the Status option to move to (F35; default: first option named like "in progress")
    "linear_state": "<state id>" | null     // linear: target started state (F35; default: the team's first `started` state)
  },
  "refresh_secs": 600,               // 300 | 600 | 1800 | 3600
  "group_by_milestone": true,        // github (F19); default on when the repo has open milestones
  "task_states": { "<Linear state id>": "backlog"|"ready"|"in_progress"|"review"|"done" } | null  // linear only (F25), today's mapping moved as is
}

ReadyFilter =                        // same `kind` tag as Tracker; ready.kind MUST equal tracker.kind
  { "kind": "github", "labels": ["ready"], "milestone": "v2" | null, "board_status": "Todo" | null, "issue_type": "Feature" | null }
| { "kind": "linear", "state_types": ["unstarted"], "label": null, "project": null, "cycle": "current" | "next" | null }
| { "kind": "local" }

MigrationNotice = {                  // F16; null when none or dismissed
  "kind": "merged" | "different_repo" | "mismatch" | "backlog",
  "from": { "task_backend": <old TaskBackendConfig json>, "intake_source": <old IntakeSource json> },
  "kept": Tracker,
  "alternative": Tracker | null,     // different_repo: the intake repo; mismatch: the other kind
  "dismissed": false
}
```

| Command | In | Out |
|---|---|---|
| `tracker_settings` | `project` | `{ settings: TrackerSettings, migration_notice: MigrationNotice \| null }` |
| `tracker_settings_set` | `project, settings` | same as `tracker_settings` (resolves a `mismatch` notice; triggers a refresh) |
| `tracker_preview` | `project, settings` (dry run, nothing saved) | `{ roots, items, ready, sample: [WorkNode + breadcrumb] (≤ 5; Local: the matched file names), error: SourceError \| null, warning: "gh_sub_issues" \| null }` |
| `tracker_options` | `project, tracker` (partial: what is filled so far) | GitHub `{ labels[], milestones[], issue_types[], boards: [{owner, number, title, statuses[]}] }` · Linear `{ teams: [{id, key, name}], projects: [{id, name, team}], labels[], states: [{id, name, type}] }` (F17; errors as `SourceError`) |
| `tracker_notice_dismiss` | `project` | `()` |

**Migration** (store migration + first read, notes §10.6): same repo / team → `merged`; same kind, different repo → keeps the task backend's repo, intake filter → Ready filter, `different_repo` with the intake repo as `alternative`; intake GitHub vs backend Linear (or reverse) → `mismatch`, the tracker is the **task backend's** until the user picks; intake `repo_docs` with a **Local** backend → Local with `backlog_glob`, `backlog`; intake `repo_docs` with a **remote** backend → `mismatch`, tracker = the remote backend until the user picks, `alternative` = Local with the backlog glob. Ready defaults when there was no intake filter: GitHub label `ready` if the repo has it else `[]`; Linear `["unstarted"]` + `cycle: "current"`. Hidden and started marks of `intake_items` are kept. `tracker_settings_set` with `ready.kind != tracker.kind` fails with field error `ready` ("Ready filter does not match the tracker"); when the user switches kind, the UI resets `ready` to that kind's defaults.

## 2. The tree (B2, B3)

```jsonc
WorkNode = {
  "id": "gh:owner/name#12" | "gh-milestone:owner/name/3" | "lin:ENG-42" | "lin-project:<id>" | "lin-milestone:<id>" | "lin-initiative:<id>" | "local:<path>" | "project:<project id>",
  "project": "<project id>",
  "kind": "issue" | "milestone" | "linear_project" | "linear_milestone" | "initiative" | "local",
  "ref": "#12" | "ENG-42" | "v2" | "dark-mode.md",
  "title": "…", "url": "https://…" | null,
  "parent": "<id>" | null, "depth": 1,            // levels count from the project row (0); roots are 1 (notes §2.4)
  "children": { "total": 2, "open": 1, "done": 1 },  // team tasks EXCLUDED (F31)
  "team_tasks": 4,                                // `orch` sub-issues written back, for the fold (F31); 0 when none
  "upstream": {
    "state": "open" | "closed" | "<Linear state name>",
    "state_type": "backlog"|"unstarted"|"started"|"completed"|"canceled" | null,
    "closed_reason": "completed" | "not_planned" | null,
    "issue_type": "Epic" | null,
    "labels": [], "assignees": [], "milestone": "v2" | null, "cycle": "Cycle 14" | null,
    "priority": 0..4 | null, "estimate": 3 | null, "target_date": "2026-12-01" | null, "due_on": "2026-12-01" | null,
    "board_status": "Todo" | null,
    "blocked_by": [DepRef], "blocking": [DepRef],
    "linked_prs": [{ "number": 61, "title": "…", "url": "…", "state": "open"|"merged"|"closed" }],   // F33
    "deleted": false,                              // F13: kept in cache only while an app link exists
    "updated_at_ms": 0
  },
  "app": AppState,
  "rollup": Rollup | null,                         // parents only, descendants, excl. team tasks and hidden (F2)
  "ready": false, "hidden": false,                 // ready = matches the Ready filter (F1: ready / closed are derived, not AppState)
  "writeback": { "status": "ok"|"pending"|"failed", "what": "<history kind>", "at_ms": 0, "message": null } | null,  // F14
  "actions": { "team": Action, "solo": Action, "program": Action, "hide": Action }   // F40; rules = notes §5.1
}
DepRef = { "id": "<WorkNode id>" | null /* null = outside this tracker */, "ref": "#13" | "acme/api#4", "title": "…", "url": "…", "open": true }
Action = "ok" | "hidden" | { "disabled": "<tooltip copy>" }
       // program: also { "open_program": "<program id>" } when the node is already a program root (notes §5.1)
AppState =
  { "state": "none" }
| { "state": "queued",  "program": { "id", "title" }, "waiting_on": ["<feature slug>"] }   // waiting_on ≠ [] → "waiting"
| { "state": "running", "feature": "<slug>", "run": "<run id>", "phase": "spec"|"planning"|"executing"|"integrating"|"pr" }
| { "state": "solo",    "agent": "<agent id>", "name": "fix-feed" }                        // F34
| { "state": "pr",      "feature": "<slug>", "pr": { "number": 61, "url": "…" } }
| { "state": "done",    "feature": "<slug>" | null, "merged_at_ms": 0 }
| { "state": "program_root", "program": { "id", "title" }, "merged": 0, "total": 3 }
Rollup = { "running": 1, "pr": 0, "waiting": 1, "queued": 1, "merged": 1, "failed": 0, "skipped": 0 }   // one word list and order (notes §2.6)

Breadcrumb = [{ "id": "<WorkNode id>", "ref": "#12", "title": "Checkout v2" }]   // root first (project row excluded), ≤ 3 + "…" collapse done by the UI (F5)

WorkFilter = {                                    // F3
  "state": "open" | "closed" | "all", "only_ready": false, "only_not_started": false,
  "labels": [], "assignees": [] /* "@me" allowed */, "milestone": null, "cycle": null | "current" | "next",
  "issue_types": [], "linear_projects": [], "include_team_tasks": false, "show_hidden": false
}
SourceStatus = { "status": "ok"|"refreshing"|"error", "error": SourceError | null, "warning": "gh_sub_issues" | null,   // F26
                 "offline": false, "refreshed_at_ms": 0,
                 "counts": { "items": 14, "roots": 3, "ready": 4 } }   // F15; counts = cached non-team-task nodes (project row, details status line)
SourceError  = P4.5 `{ code, message, fix?, retry_at_ms? }` + codes "linear_complexity", "gh_write", "linear_write"     // F27
```

| Command | In | Out |
|---|---|---|
| `work_tree` | `scope: {projects: [ids]}`, `parents: [ids] \| null` (null = roots; several = restore / reveal in one call, F37), `filter: WorkFilter`, `after: cursor \| null` (F18; only valid with `parents` null or exactly one id: it continues that list from the cursor `more` returned for it) | `{ nodes: [WorkNode], more: { "<parent id \| 'roots'>": { left: n, cursor } }, status: { "<project>": SourceStatus } }` — from the cache only. Roots paged 100, children 50. A filtered tree keeps the ancestors of matches, flagged `"ancestor_only": true` |
| `work_search` | `scope, query, filter` | `{ nodes: [WorkNode + { breadcrumb, match: { field: "ref"|"title"|"body", snippet: "…" \| null } }], total, hidden_by_filters }` (`snippet` ≤ 80 chars of plain body text around the hit, for "in body: "…"") (≤ 100; ref exact first, then title, then body) |
| `work_node` | `id, opts: { children: bool }` (F36; default `false`: no children; `true` adds `children: [WorkNode]`, first page of 50, for a node the tree has not loaded) | `WorkNode + { breadcrumb, body, body_clipped, history: [History] }` |
| `work_reveal` | `id` | `{ chain: [ids root→node], nodes: [WorkNode] }` (the ancestors and their siblings, F37) |
| `work_refresh` | `project \| null` | `()` returns at once; one in-flight refresh per project, callers join (F38); pushes `work.changed` |
| `work_hide` / `work_unhide` | `id` | `()` |
| `work_prefill` | `id, mode: "team" \| "solo"` | P4.5 `intake_prefill` shape + `origin: { id, ref, title, url, breadcrumb, tracker }`, `open_sub_issues: n`, `blocked_by_open: [DepRef]`, `ran_before: { feature, merged_at_ms } \| null`, `write_back_line: "…" \| null` (notes §5.2) |

`History = { "at_ms": 0, "what": "started"|"program_queued"|"spec_approved"|"pr_opened"|"merged"|"skipped"|"writeback_done"|"writeback_failed"|"returned", "feature": "<slug>" | null, "program": {id, title} | null, "detail": "…" | null }` (F6; copy in F6 / notes §3).

**Closed items in the cache**: sub-issues of loaded parents in every state, plus closed (GitHub) / completed or canceled (Linear) items updated in the last **90 days**; older closed history is not read. A full read on first load and after a settings save, then incremental (`since`) with an hourly full id sweep.

Starting from a node goes through the existing launch / new-session commands with the P4.5 `intake` link replaced by `"source": "<WorkNode id>"`; the run then carries:
`run.source = { node, ref, title, url, breadcrumb, tracker: "github"|"linear"|"local", program: {id, title, index, total} | null, deleted: false, closed_upstream: false, write_back: "on"|"off", writeback: WorkNode.writeback | null }` (F23; `writeback` carries the last write-back failure for the Feature header Retry), in `now` run rows and the Feature pane. Task records keep `run` (F24) so `T-n` links open `openFeature(run, {task})`; task hints / sync-conflict outcomes carry `run` and `tracker` ("github"|"linear"|"local") so the UI names the tracker without a lookup.

## 3. Ready lane in `now` (B2)

`now.intake` keeps its P4.5 shape; each item gains `node` (WorkNode id), `breadcrumb` (Breadcrumb), `actions` (as WorkNode). The lane lists exactly the nodes with `ready && !hidden && app.state == "none"` (notes §8); project status lines come from the per-project `SourceStatus`.

## 4. Programs (B4)

```jsonc
Program = {
  "id": "prg-…", "project": "…", "title": "Checkout v2", "root": ["<WorkNode id>"],
  "root_ref": "#12",                               // one root: its ref. Several siblings: their common parent's ref ("#12"), or the project name when they are roots; title then defaults to "<parent title> · 3 selected"

  "status": "draft" | "running" | "paused" | "done" | "cancelled",
  "grouping": "child" | "grandchild" | "single", "max_parallel": 2, "sequential": false,
  "features": [ProgramFeature],
  "edges": [{ "from": "pf-1", "to": "pf-2", "source": "tracker"|"sequential"|"user", "items": ["gh:…#13", "gh:…#14"] }],   // F10: `to` waits for `from`
  "cut": [["pf-2", "pf-3"]],                       // edges the user removed; kept on re-plan (F10)
  "new_items": ["<WorkNode id>"],                  // F7
  "estimate": { "low_pp": 10, "high_pp": 17, "room_pp": 12, "room_scope": "shop-api", "over_room": "none"|"high"|"low" },   // F9
  "left_out": { "closed": ["<id>"], "running": [{ "id", "feature" }] },
  "created_at_ms": 0, "started_at_ms": 0 | null, "done_at_ms": 0 | null
}
ProgramFeature = {
  "key": "pf-1", "items": ["<WorkNode id>"], "slug": "checkout-address", "title": "…",
  "mode": "team" | "solo", "team": "<team id>" | null, "brief": "markdown", "base": "main",   // F10 base
  "depends_on": ["pf-0"], "skip": false,
  "estimate": { "low_pp": 4, "high_pp": 7, "basis": "…", "rough": false },
  "state": "planned"|"waiting"|"queued"|"running"|"pr"|"merged"|"skipped"|"failed",
  "phase": "spec"|"planning"|"executing"|"integrating"|"pr" | null,
  "run": null, "agent": null, "pr": { "number", "url" } | null, "merged_at_ms": null, "actual_pp": null,
  "flags": ["closed_upstream" | "new_dependency" | "deleted_upstream"]
}
PlanIssue = { "kind": "cycle"|"already_running"|"closed"|"no_team"|"over_room", "blocking": true,
              "features": ["pf-…"], "items": ["<id>"], "feature": "<slug>" | null, "edges": [["pf-2","pf-3"]], "message": "…" }   // F8
```

| Command | In | Out |
|---|---|---|
| `program_plan` | `roots: [ids]` (one node, or ≥ 2 siblings), `grouping`, `opts: {sequential, max_parallel, cut?}` | `{ program: Program (draft), issues: [PlanIssue] }` |
| `program_start` | `program` (the edited draft) | `{ program }` (running) or `{ issues }` (dialog stays) |
| `program_update` | `id, patch`: `{pause}` · `{resume}` · `{skip: key}` · `{add_item: id, mode, team, after: [keys]}` · `{ignore_item: id}` · `{max_parallel: n}` · `{cancel}` | `Program` |
| `program` | `id` | `Program` with live states |

**Engine**: a feature starts when every `depends_on` (not skipped) is `merged`, the program is `running`, fewer than `max_parallel` of its features run, and the Governor admits it (else `queued`, held by the Governor; `waiting` = a dependency not merged). Starting = the same path as a team run / solo session started from the node (spec / PR gates per feature unchanged). Upstream changes (notes §7.3): new child → `new_items` + decision; child closed → flag + decision; dependency added → edges recomputed for not-started features + decision.

**In `now`** (F21, F32, F39): lane order `needs_you, running, programs, waiting, queued, ready, done`. `now.programs` = `{ upsert: [ProgramRow], remove: [ids] }` in deltas (whole list in the snapshot). `ProgramRow = { id, project, title, root_ref, status, counts: Rollup-like {merged, running, pr, waiting, queued, failed, skipped}, estimate: {low_pp, high_pp}, features: [{ key, slug, title, state, mode, team, phase?, pr?, waiting_on?, flags }] }`. Done / cancelled → an outcome in Done today (`kind: "program"`). Run / queue rows' `origin` gains `program: {id, title}`.

**Decisions** (F22): kinds `program_new_item` (actions `add`, `ignore`), `program_item_closed` (`skip`, `keep`), `program_dependency_added` (`open`), `program_done` (`open`), `program_feature_failed` (`open`, `skip`), `writeback_failing` (`settings`, `turn_off`). Spec / PR decisions of a program feature gain `program: { id, title, index, total }` (F29).

**Events**: `work.changed {project}`, `program.started | feature_started | feature_merged | done | cancelled {id}`, `writeback.done | failed {node, what, code?}`.

## 5. Write-back (B5)

Only when `write_back.enabled` and the tracker is GitHub / Linear; every write audited (FR-709), only items of the project's tracker.

| When | GitHub | Linear |
|---|---|---|
| A feature / solo session starts from a node | label `running_label` (no board) or board Status `board_status`; comment "Started in Orchestrator: feature <slug>"; assign the user if `assign_me` | `issueUpdate { stateId: linear_state, assigneeId? }`; `commentCreate` |
| Team task created | `orch`-labelled issue added as **sub-issue** of the source item (`addSubIssue`) | `issueCreate { parentId }` labelled `orch` |
| PR opened | PR body contains `Closes #n` (`Closes owner/name#n` when the tracker repo is not the PR's repo) | branch / title carry the identifier |
| PR merged | the core checks the item and closes it itself when still open (GitHub only auto-closes for PRs into the default branch / same repo); running label removed | moved to the team's `completed` state if Linear's automation did not |
| Every child of a program root done (single-root programs only) | root closed | root moved to `completed` |

Team-task fallback: when GitHub refuses `addSubIssue` (no sub-issues API / permission), the task is created as today (top-level `orch` issue) with a body line `Sub-task of #13`; the tree attaches it to #13 by that line, `writeback` stays `ok`. Failure → `WorkNode.writeback.status = "failed"`, history `writeback_failed`, event `writeback.failed`; `writeback_retry(node)` command; 3 failures in a project → `writeback_failing` decision. Copy: notes §10.4 (owner of the sentences), S15.

**Agents** (orch MCP): `read_issue(ref)` accepts `#12`, `owner/name#12`, `ENG-42`, and returns `parent`, `children` (non-team-task, with state), `blocked_by`, `blocking`; `update_issue` accepts Linear identifiers. A lead's first turn names its run's source item and its open children.

## 6. UI-only state

- View prefs per scope (F4): `settings_set("work.view.<sorted project ids>", { filter, toggles, expanded: [≤ 500 ids], focused, details_open, focus_root })`.
- Removal toast once (notes §1.3): `settings_set("work.board_notice_seen", true)`.

## 7. Removed (Q2)

**When**: an old command stays callable (unchanged where its data still exists, else a thin adapter over the new model) until the front task that replaces its callers has merged; **I1** removes them all, with the Board's commands, once F1–F5 are merged. The integration branch never ships a UI calling a missing command.

Commands `intake_source`, `intake_source_set`, `intake_preview`, `intake_options`, `intake_item`, `intake_hide`, `intake_unhide`, `intake_prefill` (replaced by `tracker_*` / `work_*`), the task backend settings commands (into `tracker_settings_set`), and the Board pane. `intake_refresh` becomes `work_refresh`. Brief decisions of the Overwatch (`brief_act`, `intake_prefill` with a brief) keep working through `work_prefill`'s sibling `brief_prefill(decision, mode)` (same output without `origin`; owned by B3).
