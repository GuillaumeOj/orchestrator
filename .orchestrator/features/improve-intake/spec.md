# Improve Intake · Feature spec

> Feature `improve-intake` · Integration branch `orch/improve-intake/integration` · Team: lead, **ux**, **ui** (no backend / frontend node) · Builds on [P4.5 Intake](../p-4-5-intake/spec.md), [fix-fetch-issues-from-github](../fix-fetch-issues-from-github/spec.md), [fix-linear-sync](../fix-linear-sync/spec.md), SPEC FR-501–506, FR-1302, FR-1312, ADR-004.
> Companions: [ux-brief.md](ux-brief.md) (design pass) · [api-contract.md](api-contract.md) (the model and commands the design draws, for the build feature that follows).

## 0. What this feature delivers

The team has two designers and no developers. This feature delivers the **product spec, the data contract and the full design** (UX notes, FigCraft boards, exports) for "the tracker is the source of truth". The **build** (core, store, UI code) is a follow-up feature (`improve-intake-build`, back + front), started from these docs once the design is Ready for dev. Nothing in `crates/` or `ui/` changes here.

## 1. Problem (HEAD f841c019)

Tasks and issues show as boards in three places, each with its own list and its own rules:

| Where | What it lists | Source |
|---|---|---|
| **Board** pane (`BoardPane.svelte`, from a feature's `b`, the Run inspector, the palette) | Every open issue of the repo / Linear team **plus** the team tasks the leads created (`orch`-labelled), in five status columns, filterable by feature | Task backend (`orch-tasks` GitHub / Linear, `TaskService::sync`) |
| **Mission control › Ready to start** (`mission.js` `readyLane`) | Up to 50 open issues matching one filter (a label, a milestone or a Projects v2 board; a Linear team + project / label), minus `orch` ones | Intake source (`projects.intake_source`, `intake_items`), a separate setting |
| **Feature** pane › Tasks | The run's team tasks by status | Store `tasks` of the run |

So:
1. **Two settings for one tracker.** The project's task backend and its intake source both point at the same GitHub repo or Linear team, configured twice, synced by two loops, cached in two tables.
2. **The Board mixes two kinds of things**: the user's real work items (issues written by people) and the leads' internal decomposition (`orch` tasks). Neither reads well.
3. **Everything is flat.** No list knows about GitHub **sub-issues**, Linear **projects, milestones, sub-issues**, or issue dependencies. An epic and its ten sub-issues are eleven unrelated rows (or, in Ready to start, only those carrying the label).
4. **Browsing is not possible.** Ready to start shows a 50-item filtered slice; nothing lets the user look through a project's issues, open a parent, and pick what to run.
5. **One item = one run, and only by hand.** There is no way to run a whole epic, milestone or Linear project: the app can't plan several features from an issue tree, order them by dependency, and drive them to merged PRs.

## 2. Goal and exit criterion

**The remote tracker (GitHub or Linear) is the source of truth for what work exists; the app is where the user browses it and runs teams on it.**

1. One **tracker** per project (GitHub repo, or Linear team(s)) replaces the task backend + intake source pair; the app reads it as a **tree**.
2. A **Work browser**, opened from Mission control (all projects) or a project, shows that tree with the app's own state on each node (not started · queued · running · PR open · done), with search and filters.
3. From any node the user starts a **solo session**, a **team run** (one feature), or, on a node with children, a **program**: several features planned from the children, ordered by their dependencies, run under the Governor until every PR is merged.
4. Each surface shows one kind of thing: Work browser = tracker items; Feature = its team's tasks; Mission control = what is running / waiting / ready now.

**Exit criterion (checked in the build feature):** a GitHub epic with 3 sub-issues (one blocked by another) and a Linear project with 2 milestones are each started as a program from the Work browser, with one approval; every resulting feature reaches a merged PR in dependency order, the tracker shows the items closed / done, and the user never opened the Board or an agent pane.

## 3. Product scope

### 3.1 The tracker (one setting per project)
- Project settings › **Tracker** (replaces the Task backend and Intake sections): **Local** (the app's store; optional repo-docs backlog folder, today's `repo_docs` source) · **GitHub** (repo, defaults to the project's remote) · **Linear** (one or more teams; optionally limited to some projects).
- **Ready filter** (today's intake filter, moved inside the tracker): what Mission control's *Ready to start* lane shows. GitHub: label(s), milestone, Projects v2 board + status, issue type. Linear: state types (unstarted / backlog), label, project, cycle (current / next). Default: GitHub label `ready` if it exists else none; Linear "unstarted in the current cycle".
- Migration from today: a project whose task backend and intake source name the same repo / team becomes one tracker; a different pair keeps the task backend's repo and turns the intake filter into the Ready filter; a mismatch (intake GitHub, backend Linear) is surfaced once in settings for the user to pick. The existing caches are migrated, started marks kept.

### 3.2 The tree
What the browser shows per tracker (api-contract §1):
- **GitHub**: Milestones (optional grouping) → issues → **sub-issues** → sub-sub-issues (GitHub allows 8 levels, 100 children per parent). Each issue: number, title, state (open / closed + reason), issue type (Epic, Feature, Bug, Task… when the org uses them), labels, assignees, milestone, sub-issue progress (`n / m`), **blocked by / blocking** dependencies, linked PRs, Projects v2 status (when a board is chosen in the Ready filter). Issues without a parent are roots; pull requests are not items.
- **Linear**: Initiatives (when the workspace has them, read-only grouping) → **Projects** → **Project milestones** → issues → **sub-issues** (any depth). Plus cycles as a filter / grouping, state (with its type), priority, estimate, labels, assignee, `blocks` / `blocked by` relations, project status and target date.
- **Local**: flat list (store tasks without a run + repo-docs backlog), no hierarchy.
- `orch`-labelled items (team tasks written back, §3.6) are **children of their feature's root item**, shown collapsed under a "Team tasks" fold, never as work to start.
- The tree is cached (like `intake_items`) so the browser opens at once; refreshed in the background (10 min default, on open when older than 60 s, on `r`), incrementally (`updated_at` / `since`), within GitHub / Linear rate and complexity limits (api-contract §4).

### 3.3 The Work browser
- A pane **Work**: opened from Mission control (header button and key, all projects or the filter's projects), from a project (Project pane, sidebar), from the palette. Replaces the **Board** pane, which is removed (its team-task use moves to the Feature pane, §3.5).
- Layout (ux decides the final form): a **tree / outline** of the tracker (expand, collapse, `j/k`, `h/l`), with a **details panel** (rendered body, children summary, dependencies, linked PRs, the app's history with the item); optionally a **board** view of the same scope grouped by tracker status.
- **App state on every node**, computed by the core from its links: *not started* · *ready* (matches the Ready filter) · *queued in program P* · *running · feature <slug>, phase* · *PR open #n* · *done (merged)* · *closed upstream*. A parent shows the roll-up of its children ("3 of 5 done · 1 running").
- **Search** (title, ref, body) and **filters**: state, label, assignee, milestone / cycle, issue type / Linear project, "only ready", "only not started", "hide closed".
- **Actions** on a node (each through a dialog the user submits, never one click): **Start solo** · **Start team run** (one feature for this item, and its sub-issues as the scope of the brief) · **Run as program** (only on a node with open children: §3.4) · **Open in GitHub / Linear** · **Hide from Ready** (today's "Not now"). Multi-select (`x`) of siblings → **Run selected as program**.
- Mission control's **Ready to start** lane stays the short list of the Ready filter (rows unchanged, plus a parent breadcrumb "in #12 Checkout v2") with **Browse all** to the Work browser scoped to that project.

### 3.4 Programs (run a whole epic / milestone / project)
A **program** is a set of features planned from one tree node (or a multi-selection), run in dependency order.
1. **Plan**: the core proposes one feature per **child** of the chosen node (configurable: per child, per grand-child, or the whole node as one feature), skipping closed and already-running items. Each planned feature: slug, title, mode (team / solo), team, brief (the item and its own sub-issues), base, estimate (P4.5 §3.4, summed for the program).
2. **Order**: edges from the tracker's dependencies (GitHub blocked-by, Linear `blocks`), inherited by the planned features; sub-issue order breaks ties. Cycles are reported in the plan and must be cut by the user. Optional sequential mode ("one after the other, in tree order").
3. **Review**: a **Program plan** dialog (ux designs it): the list / graph of features with their order, mode, team, estimate, total vs the week's room (the project's allocation, as P4.5's over-room warning), max features running at once (default 2), and toggles per feature (skip, solo / team, merge into the previous). The user edits and **Start program**: that one submit is the start for all of them (FR-1312 "starting stays the user's" holds: the user approved the plan).
4. **Run**: a feature starts when every feature it depends on has its PR **merged** (its base then contains the dependency's work), within the max-at-once and under the Governor (queued features show in Mission control *Queued* with "waiting for <dep> to merge"). Each feature keeps its own spec gate and PR gate (the user still approves each spec and each PR; ux may propose a batched approval card for specs).
5. **Track**: a program is one row in Mission control (title, source node, "2 of 5 merged · 1 running · 2 waiting", its features as sub-rows) and has a **Program** view (the plan with live state, edit: pause, skip, add a new child item that appeared upstream, cancel). A program ends when every feature is merged or skipped; its outcome is posted in the chat.
6. Changes upstream while running: a new child appears → offered to add (card, not automatic); a child closed upstream → its feature is flagged, not stopped; a dependency added → the order is recomputed for features not yet started.

### 3.5 Feature pane: team tasks only
- The Feature pane's Tasks section becomes the only place team tasks show (a compact board of the run's tasks, with the moves the Board allowed: assign, status, edit, open upstream). It shows the feature's **source item** (link, breadcrumb in the tree) at the top.
- The **Board pane is removed**; its key and palette entry open the Work browser. Links that opened the Board on a task (sync conflict outcomes, waiting chips `T-n`) open the Feature pane on that task.

### 3.6 Write-back to the tracker
P4.5 D3 kept intake read-only. With the tracker as source of truth, the app reflects runs back, **per project, opt-in, default on** for GitHub and Linear (see §6 Q1):
- Start → the root item is assigned to the user (option) and moved to *In progress* (Linear state type `started`; GitHub: Projects v2 status when a board is configured, else a label `orch:running`), with a comment "Started in Orchestrator: feature <slug>".
- Team tasks → created as **sub-issues** of the root item, labelled `orch` (today they are top-level `orch` issues), so the tree shows them under their feature (§3.2).
- PR opened → linked (`Closes #n` in the PR body for GitHub; Linear's PR link by identifier in the branch / title).
- PR merged → the item closes (GitHub, by `Closes`) / moves to *Done* (Linear, by its PR automation or the core). Program parent: closed when every child is done (GitHub does not auto-close parents).
- Every write goes through the core with the user's credentials (ADR-009), is audited, and never touches items outside the project's tracker.

### 3.7 Agents and the tracker
- `read_issue` / `update_issue` (orch MCP) gain Linear identifiers (`ENG-42`) and `children` / `parent` / `dependencies` in what they return, so a lead can read the sub-issue tree of its feature's item. The lead's first turn names its source item and its children.

## 4. Out of scope
- The build (core, store, UI code): follow-up feature.
- Other trackers (Jira, Notion databases); several trackers per project; cross-project programs (a program lives in one project).
- Creating or editing tracker items from the Work browser beyond the write-back of §3.6 (create, edit, re-parent stay in GitHub / Linear; the browser has **Open in…**).
- The Overwatch starting a program (it may propose a brief, P4.5 §3.5; starting stays the user's).
- Auto-merging PRs; programs skipping the per-feature spec / PR gates.

## 5. Design deliverables (this feature's tasks)
- **ux** (`docs/design/improve-intake-design-notes.md`): information architecture of the three contexts, the Work browser (tree, details, filters, search, app states, keys), the program plan dialog and the program row / view, Ready lane changes, tracker settings and migration notice, Feature pane tasks, removal of the Board, write-back settings and its visible marks, every state (loading, empty, rate limited, scope missing, Linear complexity, offline cache, huge trees), copy, keys (page 05 conflict check). A "Findings for lead" table for contract gaps.
- **ui** (FigCraft, the Google Drive file): boards for each of these at 1512 and 1280, Light copy where relevant, components on 02 (tree row, state chip, program row), exports `.orchestrator/features/improve-intake/design/ii-*.png`, `docs/design/README.md` rows **In review**.
- The design is reviewed by lead against this spec and the contract, then by the user (Ready for dev) before the build feature starts.

## 6. Decisions for the user (proposed defaults; approving this spec accepts them)
| # | Question | Proposed |
|---|---|---|
| Q1 | Write back to GitHub / Linear (status, sub-issues for team tasks, close on merge)? | **Yes, per project, default on**; read-only remains a choice. |
| Q2 | Remove the Board pane? | **Yes**: tracker items in the Work browser, team tasks in the Feature pane. |
| Q3 | Program granularity default | **One feature per direct child**; the plan dialog can merge or split. |
| Q4 | Program start | **One approval of the plan** starts all features over time; spec and PR gates stay per feature. |
| Q5 | Max features of a program running at once | **2** (setting per program). |
| Q6 | Repo-docs backlog | Kept as the **Local** tracker's backlog folder; no hierarchy. |

## 7. Acceptance of this feature (design)
1. `spec.md`, `ux-brief.md`, `api-contract.md` approved; `improve-intake-design-notes.md` covers every point of ux-brief §2 with copy, states and keys, and its findings are settled by lead (in `decisions.md`).
2. FigCraft boards for every surface of §5, at 1512 and 1280, exported under `design/`, listed in `docs/design/README.md` (In review); the Cover updated.
3. Lead review: every app state, action and source of §3 is drawn; no screen mixes tracker items and team tasks; every start is a dialog.
4. The build feature's spec can be written from these docs without a new design question.
