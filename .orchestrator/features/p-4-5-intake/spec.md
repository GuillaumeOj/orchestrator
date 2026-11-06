# P4.5 — Intake · Feature spec

> Feature `p-4-5-intake` · Integration branch `orch/p-4-5-intake/integration` · Source: [docs/SPEC.md](../../../docs/SPEC.md) §11 P4.5, FR-1312, §7 `Project.intakeSource`, FR-1302 (lanes), FR-809 (cost of Run now), ADR-011 §4 ("never start work … tasks from intake") · Why: [brief 2026-10-28](../../../docs/brief/2026-10-28-overwatch.md) "Intake: per project … starting work stays yours"
> Companions: [api-contract.md](api-contract.md) (core ↔ UI, core ↔ Overwatch) · [ux-brief.md](ux-brief.md) (design pass, copy, states) · [decisions.md](decisions.md)
> Builds on: P4.2 Mission control ([p-4-2-mission-control](../p-4-2-mission-control/spec.md)), P4.3 cards, P4.4 the Overwatch ([p-4-4-the-overwatch](../p-4-4-the-overwatch/spec.md); `propose_rule` → `rule_proposal` card is the pattern for `propose_brief` → `brief` card).

## 1. Goal and exit criterion

SPEC §11: *"Intake source per project and the Ready to start lane (FR-1312)."* FR-1312: GitHub (label, milestone or project), Linear (team, project or label), docs in the repo (a glob), or none. Items appear in Mission control as **Ready to start** with an estimated cost; starting one (solo session or team run) is always the user's action. With no source, **Plan with the Overwatch** turns a conversation into a feature brief the user then launches.

**Exit criterion: a week's work started from intake items only.** Checked by (a) an end-to-end test with fake `gh`, fake Linear transport and a repo with a backlog glob: items of each source show in `now.intake`, a team run and a solo session started from two of them leave the lane and carry their item, a closed issue leaves the lane, a planned brief from the fake Overwatch launches a run; (b) after merge, one real week where every run and solo session is started from an intake item or a planned brief: `orchd intake --week` prints "started from intake: n of n" and the number is recorded in `decisions.md`.

## 2. What the code does today (HEAD 5882f8b)

| Item | Today |
|---|---|
| Project model | `projects` table (migration 1, `migrations.rs:17`), latest migration **16**. `Project` (`orch-store/src/model.rs:17`): `task_backend` and `settings` JSON. `TaskBackendConfig` (`orch-store/src/teams.rs:446`): Local / Github{repo?} / Linear{team, project?, states}. `Store::update_project_settings` (`teams.rs:1232`). No intake field (SPEC §7 reserves `intakeSource`). |
| GitHub | Core-only `gh` CLI with the user's login (`orch-git/src/gh.rs:295`, 30 s deadline). `orch-tasks/src/github.rs` lists only issues labelled `orch` (the task backend); `GithubIssues::list` (`:338`) pages REST. `github_issue.rs` has `read_issue` and `known_repo(p)` (`:81`). Nothing lists by user label, milestone or Projects v2. |
| Linear | GraphQL via `LinearHttp` (`agent-runtime/src/linear.rs:43`, Keychain `linear`), `orch-tasks/src/linear.rs` (`teams` `:139`, `team_states` `:153` with projects, `LinearIssues::list` `:487` filters `orch` label, 429 back-off). |
| Repo files | `git cat-file blob <rev>:<path>` used by `artifacts.rs:179`; no `ls-tree`, **no glob crate** in the workspace. |
| Sync loops | Task sync every 5 min for watched projects only (`runtime.rs:373`, `team.rs:928`); git fetch every 60 s for projects active in 7 days (`server.rs:20`, `orch-git/src/fetch.rs`). |
| Mission control | Pure render of `now` (`MissionPane.svelte`; test asserts no `invoke`). Lanes in `ui/src/lib/mission.js:258` (`needs_you, running, waiting, queued, done`), rows by `kind` (`run, solo, queue, outcome, decisions`). Core: `now_snapshot` / `delta` (`now.rs:411`, `:113`). |
| Cost estimates | Only **per turn**: `QueueEntry.estimated_cost_pct` = `avg_turn_cost / k` (`governor.rs:1730`, `:1867`), shown as `run_now {week_pp, five_hour_pp, binds}` (`attention.rs:193`). The weekly report has historical `by_feature` usage (`report.rs:102`). No item-level estimate. |
| Starting work | Solo: `agent_start(NewSession{project_id, name, model, effort, permission_mode, base, message, tier})` (`orch-app/src/core.rs:143`, `:436`); `message` is the first turn. Team: `team_launch(LaunchSpec{project_id, team_id, slug, title, brief, base, off_hours})` (`team.rs:51`, `:313`); the brief goes to the lead's first turn (`lead_brief` `:435`, with `brief_hint` for `#issues`). Dialogs: `new-session` and `launch` in `ui/src/components/Dialogs.svelte`. |
| Overwatch | Tools `world … propose_rule, post` (`overwatch/tools.rs:63`); role.md forbids starting work. No brief proposal, no card that opens a launch dialog. |
| Project settings | `ProjectSettingsPane.svelte` sections general (placeholder), files, backend, allocation, notion, mcp; `project_settings` / `project_settings_set`. |

## 3. Scope

### 3.1 Design pass (M) — ux, then ui
New surfaces, nothing drawn yet: the **Ready to start** lane and its rows (per source, with cost), lane states (refreshing, source error, no source → Plan with the Overwatch, many items, hidden items), row details, start actions and the **prefilled** launch / new-session dialogs ("from #123"), the run / solo row's origin mark, Project settings › **Intake** section (+ preview and errors), the Overwatch **brief** card and its lifecycle, keys (page 05), the Week planner / report line "started from intake". Notes in `docs/design/p4-5-design-notes.md`; boards in the FigCraft file (the Google Drive copy, see ux-brief); exports `design/p45-*.png`. Front's UI work starts when ui hands off; front may build the store wiring, `mission.js` lane logic and mocks before.

### 3.2 Intake source per project — FR-1312 (M) — back
1. **Config** (api-contract §1): migration **17** adds `projects.intake_source TEXT` (JSON, `NULL` = none), typed `IntakeSource` beside `TaskBackendConfig`:
   - `github { repo?, filter: label{names[]} | milestone{title} | project{owner, number, status?} }` — `repo` defaults to `known_repo(p)`. Labels: all of `names` (AND, as GitHub's `labels=`). Milestone by title (resolved to its number at refresh). Projects v2 by owner + number, optional `status` = the value of the project's *Status* field to keep (e.g. "Todo"); `gh api graphql`, needs the `read:project` scope (else a source error naming `gh auth refresh -s read:project`).
   - `linear { team, project?, label?, include_backlog }` — state type `unstarted` (and `backlog` when `include_backlog`); at least the team.
   - `repo_docs { glob, branch? }` — default glob `.orchestrator/backlog/*.md`; `branch` defaults to the project's base branch (`settings.base_branch`, else `default_branch`), read as the **local** ref (D2).
   - `none` (also `NULL`).
2. **Items** (api-contract §1.2): what each source yields, open only:
   - GitHub: open **issues** (pull requests excluded) matching the filter, **minus** issues labelled `orch` or `orch:*` (team tasks of the task backend, never intake). Title, body (first 4 000 chars), URL, `#number`, labels, assignees, milestone, updated_at.
   - Linear: issues of the team (+ project / label) in the kept state types, minus `orch`-labelled; identifier (`ENG-42`), title, description, URL, labels, `estimate` (points), priority, updated_at.
   - Repo docs: each file matching the glob on the branch (`git ls-tree -r --name-only <ref>`, matched with `globset` with `literal_separator(true)`, a new workspace dependency); content by `git cat-file blob`. Title = YAML front matter `title`, else the first `# ` heading, else the file name. Optional front matter keys: `size` (`S|M|L`), `mode` (`team|solo`), `team` (team id), `status` (`done` or `started` hides the item). The item's ref is the path; updated = the blob id changing.
   - Order in the lane: per project, the source's order (GitHub: milestone/label → oldest first; Projects v2: board order; Linear: priority then sort order; repo docs: path order, so `01-…md` naming works).
   - Cap: 50 items per project (more → "+ n more in <source>" with a link).
3. **Refresh** (`intake::Refresher`, its own thread): every project with a source, every **10 min** (setting `intake.refresh_secs`), and at once on: app start (staggered 2 s per project), a source change, `intake_refresh` (the lane's Refresh, the settings Preview), and for repo docs when the branch tip changes (checked on the existing 60 s fetch tick, no network). One refresh in flight per project; a failure keeps the last good items and records the source error (`gh` missing / not logged in, scope missing, 404 repo, Linear key missing / invalid, rate limit with retry time, bad glob, branch missing). Every refresh writes the cache and pushes on change only.
4. **Cache** (migration 17): table `intake_items (project_id, source, ref, title, url, body, meta JSON, updated_at, fetched_at_ms, state, started_kind, started_ref, hidden_at_ms, hidden_rev)`, unique `(project_id, source, ref)`. Items gone from the source are deleted unless started (kept for the report). The cache makes the lane show at start before the first refresh (marked "updated 12 min ago").
5. **Commands** (api-contract §3): `intake_source(project)`, `intake_source_set(project, source)` (validates, refreshes at once, returns the preview), `intake_preview(project, source)` (dry run: first 5 items + count + error, nothing stored), `intake_refresh(project?)`, `intake_hide(item)` / `intake_unhide(item)`, `intake_item(item)` (full body for details).

### 3.3 The Ready to start lane — FR-1302, FR-1312 (M) — back (data) + front
- **Data in `now`** (api-contract §2): `now.intake = { items[], sources[] }`, in the snapshot and replaced whole in a delta when it changed (small: ≤ 50 items × projects, bodies excluded). Mission control stays a pure render of `now` (the P4.2 test holds).
- **Lane** *Ready to start*, after *Queued*, before *Done today*; per-project filter applies. Rows: source glyph + ref (`#123`, `ENG-42`, `backlog/checkout-v2.md`), title, project, labels (≤ 2), the **estimate** (§3.4), the suggested mode (team / solo) and **Start** actions. Not part of the attention ladder: no notification, not in the badge, not in "needs you".
- A project with **no source** shows one line in the lane: "<project> has no intake source · **Plan with the Overwatch** · Set up intake" (only when the filter shows that project; with several such projects, one line listing them). A project with a source and **zero** items: "Nothing ready in <source>" with Refresh. A source error: the lane header line in error style with the reason and **Fix** (opens Project settings › Intake). Hidden items: "n hidden · Show".
- **Start** (always the user's action, through the existing dialogs, never one click):
  - **Start team run** opens the `launch` dialog prefilled: project, team = the item's `team` (repo docs) else the project's `default_team`, slug from the title (kebab, ≤ 40, unique), title, **brief** = the item rendered as markdown (title, source link, body; for GitHub the `#number` so `brief_hint` tells the lead to `read_issue`), base unchanged. The dialog shows "From #123 · <source>".
  - **Start solo** opens `new-session` prefilled: project, name from the title, message = the same text (plus "When done, say so; the item is <url>").
  - The submit carries `intake: {item}` (api-contract §3.2). On success the core marks the item `started` with `started_kind = run | agent` and its ref, emits `intake.started {item, source, mode, project}`, and the item leaves the lane. The run row / solo row shows its origin ("from #123", link) — `RunAttention.origin` / `AgentAttention.origin`.
  - Suggested mode: `mode` front matter, else **team** when the project has a default team, else **solo**. ⏎ uses the suggested mode.
- **After start**: the item stays `started` while its run or session exists. If the run is deleted or the session killed before any outcome, the item returns to Ready (`state = ready`). When the run finishes (PR merged) or the session ends, the item stays started (the report counts it). The core **writes nothing back** to GitHub, Linear or the repo in P4.5 (D3).
- **Hide** ("Not now"): the item leaves the lane until it changes upstream (`updated_at` / blob changes) or the user unhides it.

### 3.4 Estimated cost — FR-1312 (M) — back
- An item's estimate is **pp of the week** for each mode, as a range, with its basis (api-contract §2.2), recomputed on refresh and when the calibration moves by ≥ 10 %.
- **History**: the cost of each finished team run (sum of its members' attributed usage, `UsageRecord` by agent and allocation) and each ended solo session with ≥ 1 turn, over the last 8 weeks, converted to pp with the current calibration (`k`, `Calibration`). Per project when it has ≥ 3 samples of that mode, else across projects, else **defaults** (team 6 pp, solo 1.5 pp; settings) with basis `default`.
- **Size**: S × 0.5, M × 1, L × 2 from repo-docs `size`, Linear `estimate` (≤ 1 S, ≤ 3 M, ≥ 4 L) or GitHub labels `size:S|M|L` / `size/S` (case-insensitive); unknown = M. Range = p25–p75 of history × size (one sample or defaults: ± 40 %).
- Shown as "≈ 4–7 pp week" (team) / "≈ 1–2 pp" (solo), basis on hover / details ("median of 7 runs in checkout · size M"). When the high end exceeds what is left of the week's plan for the project (allocation) or of the week: a muted warning "over this week's room (3 pp left)". The estimate is advice; it never blocks a start (the Governor still queues as today).

### 3.5 Plan with the Overwatch — FR-1312 (S) — back + front
- **Entry points**: the lane's no-source line, the palette "Plan with the Overwatch…" (any project), the brief card's **Revise**. It focuses the chat input with a draft "Plan a feature for **<project>**: " and the message is sent with `context = {plan: project_id}` (`chat_send` gains `context`, api-contract §3.3), which the core passes in the Overwatch's `user_message` event with the project's path, its docs folder, its intake source (if any) and its last 5 feature briefs' titles.
- **New Overwatch tool** `propose_brief(project, title, slug?, brief, mode, team?, reply_to?)` (api-contract §4): validated by the core (project known, title ≤ 80, brief ≤ 8 000 chars markdown, slug free or derivable, team exists for `team`). It creates a decision of kind **`brief`** (level 4: never notifies, not in the badge, blocks nobody), its card posted in the chat under the conversation's thread. The Overwatch may revise: a new `propose_brief` with the same `reply_to` thread **replaces** the open brief card (the old one collapses to "revised").
- **Brief card**: rendered markdown brief, project, mode and team, the estimate (§3.4, size M); actions **Launch ⏎** (opens the `launch` or `new-session` dialog prefilled with it, mode as proposed; submit carries `brief_decision`) · **Edit e** (same dialog, focus on the brief) · **Dismiss r**. On a successful start the card closes with effect "launched <slug>" (`decided_by = user`). The Overwatch **never** starts it (ADR-011; the tool creates a card only), and role.md gains the planning paragraph (ask short questions, read the project's docs, cite them, then `propose_brief`; never claim it started anything).
- When the Overwatch is `off` / `down`, the entry points still focus the input (the message waits, P4.4) and the line says so.

### 3.6 Settings › Intake — FR-1312 (M) — front, back for the commands
- A new section **Intake** in Project settings (`ProjectSettingsPane` `SECTIONS`, after Task backend): segmented **None · GitHub · Linear · Repo docs**; fields per source (§3.2.1): GitHub repo (prefilled), filter kind (Label / Milestone / Project) and its value(s) (milestones and labels offered from `intake_options`); Linear team / project / label (from `linear_teams`, `linear_states`, `intake_options`), Include backlog; Repo docs glob (default shown), branch. **Preview** (`intake_preview`) shows the count, the first 5 titles and any error before saving; **Save** (`intake_source_set`). Missing GitHub login / Linear key links to Settings › Integrations. `ProjectSettingsPane` `openProjectSettings(project, 'intake')` is the **Fix** target.
- Global Settings › Pacing (or a small Intake block, ux decides): refresh interval, default estimates (team, solo).

### 3.7 Measuring the exit criterion (S) — back
- Events `intake.started {item, project, source, mode, est_pp}` and `intake.planned_started {decision, project, mode}` (api-contract §6); every `team_launch` / `agent_start` without either is `intake.untracked_start`.
- `orchd intake [--week]`: sources and their state, item counts per project, and this week's starts: "started from intake: 9 of 10 (1 untracked: agent fix-ci-3)".
- The weekly report gains one line "Started from intake n of m" (report JSON `intake {started, planned, untracked}`), shown on the report pane (ux places it).

## 4. Out of scope (P4.5)
- Writing back to the source (assigning, labelling, commenting, closing an issue, editing a backlog file) — D3; the task backend's own sync is unchanged.
- The Overwatch starting work, or ranking / picking items for the user.
- Several intake sources per project; GitHub search queries beyond label / milestone / project; Linear custom views; Notion as a source.
- Auto-starting items off-hours (the launch dialog's existing *Run off-hours* stays available).
- Splitting an item into several runs.

## 5. Non-functional
- **Starting stays the user's**: no path from a refresh, the Overwatch or a card starts a session or a run without the user submitting a dialog (test).
- **Read-only to sources**: intake only calls `gh api` GET / GraphQL queries, Linear queries, `git ls-tree` / `cat-file`; a test asserts no mutation is issued.
- **Cost**: refresh ≤ 1 request page per source per 10 min in steady state (GitHub `since` / ETag not required); Linear honours 429. No refresh for a project with `none`.
- **Responsiveness**: the lane renders from the cache at start; a refresh never blocks `now`.
- **Safety**: item text is untrusted (an issue body is anyone's text). It reaches agents only inside the brief the user submits, framed as "Brief from the user" with the source link; the body is shown to the user in the dialog before submit. Repo-docs paths are confined to the repo (no `..`, no symlink escape: blobs read from git, not the disk).
- **Accessibility**: the lane is a `list` like the others; the estimate is text; Start buttons have labels naming the item.
- **Tests**: `make ci` green. Core: config (de)serialization and validation per source, each source's listing and filters with fakes (`orch` label excluded, PRs excluded, Projects v2 status filter, Linear backlog flag, front matter, glob, branch tip), errors per source, cache (gone items, started kept, hide/unhide on change), refresher scheduling, estimate (history per project / global / defaults, size, range, room warning), start linkage (team / solo / return to ready on delete / kill), `propose_brief` validation and replace, brief card launch, events and `orchd intake`. UI: lane rows per source, states (no source, empty, error, refreshing, hidden, many), filter, keys, prefilled dialogs and the `intake` param, origin mark on rows, settings section with preview and errors, brief card lifecycle, palette entry. Tests hold at any date, hour and time zone.

## 6. Acceptance (demo at the PR gate)
1. Project A with GitHub label `ready`: two issues show in Ready to start with estimates; an `orch`-labelled one does not. **Start team run** opens the launch dialog prefilled with the issue; submitting starts the run, the item leaves the lane, the run row says "from #12".
2. Project B with Linear team ENG + label `next`: items show with their identifiers; **Start solo** starts a session with the issue text as its first message.
3. Project C with `.orchestrator/backlog/*.md`: committing a new file on main adds it within a minute; `size: L` doubles the estimate; `status: done` hides it.
4. A source error (gh logged out) shows in the lane with Fix → Project settings › Intake.
5. Project D with no source: **Plan with the Overwatch** → a short conversation → a brief card → Launch → dialog → run started; the card collapses to "launched".
6. Hide an item; it comes back when its issue is edited.
7. `orchd intake --week` and the weekly report line.
8. FigCraft P4.5 boards In review → Ready for dev; exports under `design/`.
9. After merge, the real week of §1 (b), recorded in `decisions.md`.
