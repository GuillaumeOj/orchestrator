# Improve Intake · API contract (core ↔ UI)

> Companion of [spec.md](spec.md). This is the model the design draws and the build feature implements. Field names are binding for the build; ux may raise gaps in its "Findings for lead" table. It extends P4.5's [api-contract](../p-4-5-intake/api-contract.md) (estimates, prefill, `intake` link on starts), which stays valid where not replaced.

## 1. Model

### 1.1 Tracker (per project; replaces `task_backend` + `intake_source`)
```jsonc
Tracker =
  { "kind": "local", "backlog_glob": ".orchestrator/backlog/*.md" | null }
| { "kind": "github", "repo": "owner/name", "project_board": { "owner": "acme", "number": 4 } | null }
| { "kind": "linear", "teams": ["ENG"], "projects": ["<slugId>"] | null }   // null = every project of the teams
TrackerSettings = {
  "tracker": Tracker,
  "ready": ReadyFilter,
  "write_back": { "enabled": true, "assign_me": false, "running_label": "orch:running" },
  "refresh_secs": 600
}
ReadyFilter =                                   // what Mission control › Ready to start lists
  { "github": { "labels": ["ready"], "milestone": "v2" | null, "board_status": "Todo" | null, "issue_type": "Feature" | null } }
| { "linear": { "state_types": ["unstarted"], "label": null, "project": null, "cycle": "current" | "next" | null } }
| { "local": {} }
```

### 1.2 WorkNode (one node of the tree)
```jsonc
WorkNode = {
  "id": "gh:owner/name#12" | "lin:ENG-42" | "lin-project:<slugId>" | "lin-milestone:<id>" | "gh-milestone:owner/name/3" | "local:<task id | path>",
  "project": "<project id>",
  "kind": "issue" | "milestone" | "linear_project" | "linear_milestone" | "initiative" | "local",
  "ref": "#12" | "ENG-42" | "Checkout v2",        // short label
  "title": "…",
  "url": "https://…" | null,
  "parent": "<WorkNode id>" | null,
  "children": { "total": 5, "open": 3, "done": 2 },  // sub-issues / issues of a milestone or project
  "depth": 0,
  "upstream": {                                      // as the tracker says
    "state": "open" | "closed" | "<Linear state name>",
    "state_type": "backlog" | "unstarted" | "started" | "completed" | "canceled" | null,
    "closed_reason": "completed" | "not_planned" | null,
    "issue_type": "Epic" | null,
    "labels": ["…"], "assignees": ["login"], "milestone": "v2" | null, "cycle": "Cycle 14" | null,
    "priority": 0..4 | null, "estimate": 3 | null, "target_date": "2026-12-01" | null,
    "board_status": "Todo" | null,
    "blocked_by": ["<WorkNode id>"], "blocking": ["<WorkNode id>"],
    "linked_prs": [{ "number": 61, "url": "…", "state": "open" | "merged" | "closed" }],
    "updated_at_ms": 0
  },
  "app": AppState,
  "ready": true,                                     // matches the Ready filter
  "hidden": false,                                   // "Not now"
  "team_task": false                                 // an `orch` sub-issue written back by a run (§3.2)
}
AppState =
  { "state": "none" }
| { "state": "queued", "program": "<program id>", "waiting_on": ["<feature slug>"] }
| { "state": "running", "feature": "<slug>", "run": "<run id>", "phase": "spec|planning|executing|integrating|pr" }
| { "state": "solo", "agent": "<agent id>" }
| { "state": "pr", "feature": "<slug>", "pr": { "number": 61, "url": "…" } }
| { "state": "done", "feature": "<slug>" | null, "merged_at_ms": 0 }
// a parent with children also gets:
"rollup": { "running": 1, "queued": 2, "pr": 0, "done": 2, "none": 0 }
```

### 1.3 Program
```jsonc
Program = {
  "id": "prg-…", "project": "…", "title": "Checkout v2", "root": ["<WorkNode id>"],
  "status": "draft" | "running" | "paused" | "done" | "cancelled",
  "max_parallel": 2, "sequential": false,
  "features": [ProgramFeature],
  "estimate": { "low_pp": 18, "high_pp": 30 }, "created_at_ms": 0
}
ProgramFeature = {
  "key": "pf-1", "items": ["<WorkNode id>"], "slug": "checkout-address", "title": "…",
  "mode": "team" | "solo", "team": "<team id>" | null, "brief": "markdown",
  "depends_on": ["pf-0"], "skip": false,
  "estimate": { "low_pp": 4, "high_pp": 7, "basis": "…" },
  "state": "planned" | "waiting" | "queued" | "running" | "pr" | "merged" | "skipped" | "failed",
  "run": "<run id>" | null, "agent": "<agent id>" | null, "pr": { "number": 0, "url": "" } | null,
  "flags": ["closed_upstream" | "new_dependency"]
}
PlanIssue = { "kind": "cycle" | "already_running" | "closed" | "no_team" | "over_room", "features": ["pf-…"], "message": "…" }
```

## 2. In `now` (Mission control)
- `now.intake` keeps its P4.5 shape for the Ready to start lane; each item gains `node` (its WorkNode id) and `breadcrumb` (`[{ref, title}]`, root first, ≤ 3).
- `now.programs: [{ id, project, title, status, counts: {merged, running, pr, waiting, queued, skipped, failed}, features: [{key, slug, state, waiting_on?}] }]`, replaced whole in a delta when it changed.
- Run and solo rows' `origin` (P4.5) gains `program: {id, title}` when started by a program.

## 3. Commands
| Command | In | Out |
|---|---|---|
| `tracker_settings(project)` / `tracker_settings_set(project, settings)` | | `TrackerSettings` + `migration_notice?` |
| `tracker_preview(project, settings)` | dry run | `{ roots: n, items: n, ready: n, sample: [WorkNode], error? }` |
| `work_tree(scope, parent?, filter?)` | `scope = {projects: [..]}`; `parent` = node id to expand (null = roots) | `{ nodes: [WorkNode], more: n, fetched_at_ms, status: SourceStatus }` (children loaded lazily per expand) |
| `work_search(scope, query, filter?)` | text over ref, title, body | `{ nodes: [WorkNode + breadcrumb] }` (≤ 100) |
| `work_node(id)` | | `WorkNode + { body, children: [WorkNode], history: [{at_ms, what, feature?}] }` |
| `work_refresh(project?)` | | `()` (pushes `work.changed`) |
| `work_hide(id)` / `work_unhide(id)` | | `()` |
| `work_prefill(id, mode)` | as P4.5 `intake_prefill` | `{ dialog: "launch" | "new-session", fields }` |
| `program_plan(roots, grouping, opts)` | `grouping = "child" | "grandchild" | "single"`; `opts = {sequential, max_parallel}` | `{ program: Program (draft), issues: [PlanIssue] }` |
| `program_start(program)` | the edited draft | `Program` (running) or `issues` |
| `program_update(id, patch)` | pause / resume / skip feature / add item / max_parallel / cancel | `Program` |
| `program(id)` | | `Program` with live states |

`SourceStatus = { status: "ok" | "refreshing" | "error", error?: SourceError (P4.5 codes + "linear_complexity", "gh_sub_issues"), refreshed_at_ms }`.
Events: `work.changed {project}`, `program.started|feature_started|feature_merged|done|cancelled`, `writeback.done|failed {node, what}`.

## 4. Reading the trackers (constraints the design must reflect)
- **GitHub**: GraphQL through `gh api graphql` (user's login, core only). Issues with `parent { id }`, `subIssues(first: 50)`, `subIssuesSummary { total completed percentCompleted }`, `issueType { name }`, `blockedBy` / `blocking` (issue dependencies), `closedByPullRequestsReferences`, `milestone`. Roots and children are paged and loaded **per expand** past the first level; 8 levels max, 100 sub-issues per parent. Projects v2 status needs `read:project` (existing `gh_scope` error).
- **Linear**: queries kept **under 10 000 complexity points** (fix-linear-sync estimator applies to every new query): `projects(first: 50) { … projectMilestones(first: 20) }`, issues with `parent { id }`, `children(first: 50) { nodes { id } }`, `relations` / `inverseRelations(first: 20)` of type `blocks`, `cycle`, `projectMilestone`, paged 25–50 per call. 429 back-off as today.
- **Cache**: one `work_nodes` table (project, id, parent, kind, upstream JSON, position, fetched_at, app links) replacing `intake_items` and the remote half of `tasks`; team tasks stay in `tasks`, linked to their written-back sub-issue by `external_id`.
- **Write-back** (spec §3.6): GitHub REST/GraphQL mutations `addSubIssue`, labels, comments, issue close; Linear `issueCreate { parentId }`, `issueUpdate { stateId, assigneeId }`, `commentCreate`. Only when `write_back.enabled`, audited (FR-709).

## 5. Keys (proposal, ux settles them on page 05)
Work browser: `j/k` move, `h/l` collapse / expand, `⏎` details, `/` search, `f` filters, `x` select, `t` start team run, `s` start solo (check against `s` suspend on agent rows), `p` run as program, `o` open upstream, `⌫` hide from Ready, `r` refresh. Mission control: `w` open the Work browser (check conflicts).

## 6. Accepted design findings (D2)
The findings F1–F40 of [improve-intake-design-notes.md §14](../../../docs/design/improve-intake-design-notes.md) are part of this contract **as proposed there**: where a finding and a section above differ, the finding wins. In short: derived `ready` / closed states (F1), roll-up scope and `waiting` (F2, F31), `WorkFilter` (F3), view prefs in settings (F4), breadcrumb with ids (F5), `history[].what` enum (F6), `Program.new_items`, `edges`, `base`, room, `PlanIssue.items` (F7–F10), `upstream.deleted` (F13), `WorkNode.writeback` + `writeback_retry` + `writeback_failing` (F14, F27), per-project `SourceStatus` with `retry_at_ms` / `offline` (F15), `migration_notice` shape (F16), `tracker_options` (F17), children paging cursor (F18), `group_by_milestone` (F19), keys (F20 replaces §5), lane order and `program` outcomes (F21), program decision kinds (F22), `run.source` (F23), task → run links (F24), Linear `task_states` (F25), `gh_sub_issues` as a warning (F26), the richer `now.programs` sent as deltas per id (F32, F39), PR titles (F33), the solo `name` (F34), the write-back target status (F35), lighter `work_node` + `work_tree(parents[])` + `work_reveal` (F36, F37), joined refreshes (F38), `WorkNode.actions` (F40).
