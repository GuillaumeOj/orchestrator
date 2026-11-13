# Implement Improve Intake · Feature spec (build)

> Feature `implement-improve-intake` · Integration branch `orch/implement-improve-intake/integration` · Team: lead, **back**, **front**, ui, ux.
> This is the **build** (`improve-intake-build`) of the design feature **improve-intake**. Its docs are the authority and are not restated here:
> - [improve-intake/spec.md](../improve-intake/spec.md) (product scope §3, decisions §6 Q1–Q6) and [decisions.md](../improve-intake/decisions.md) (D1–D4)
> - [improve-intake/api-contract.md](../improve-intake/api-contract.md) + design findings F1–F40 (binding, "the finding wins")
> - [docs/design/improve-intake-design-notes.md](../../../docs/design/improve-intake-design-notes.md) (**notes §n** below: copy, states, keys)
> - Boards: FigCraft file (Google Drive, page 02, 04.2, 04.3, 04.4, 04.7, 04.8, 05) and exports [improve-intake/design/ii-*.png](../improve-intake/design/)
>
> Companions: [api-contract.md](api-contract.md) (the consolidated, binding build contract: improve-intake contract + F1–F40 folded into one set of shapes) · [ux-brief.md](ux-brief.md) (how front uses the boards; ui / ux review).

## 0. Prerequisite (done: #67 merged, branch rebased)

The improve-intake docs and exports are on `orch/improve-intake/integration` (pushed, **not yet merged into `main`**). The build reads them from the base branch:
1. The user merges the improve-intake PR into `main`.
2. Lead brings it into this integration branch (`rebase_base`, through a docs-only gate task if the orchestrator refuses a rebase before tasks exist).
3. Only then are the code tasks created.

## 1. What this feature delivers

Everything the improve-intake design describes, in `crates/` and `ui/`:
1. **One tracker per project** (Local · GitHub · Linear) replacing the task backend + intake source pair, with the Ready filter, write-back options, Preview, options, migration of today's settings and caches (improve-intake spec §3.1, notes §10).
2. **The tree**: GitHub milestones → issues → sub-issues (types, dependencies, linked PRs, Projects v2 status), Linear projects → milestones → issues → sub-issues (cycles, priority, estimate, relations), Local flat list; cached in `work_nodes`, refreshed incrementally within rate / complexity limits (spec §3.2, contract §4).
3. **The Work browser** pane: tree + details, app state, roll-ups, team-tasks fold, depth tags and Focus here, search, filters saved per scope, multi-select, actions through dialogs, every state of notes §11, keys of notes §12.1 (spec §3.3, notes §2–§5, §11).
4. **Programs**: plan dialog (grouping, order, at once, plan issues and fixes, per-row edits), the program engine (start each feature when its dependencies are merged, within max at once, under the Governor), the **Programs** lane and row, the Program view, program cards, program tag on spec / PR cards (spec §3.4, notes §6–§7).
5. **Ready to start** lane amended (breadcrumb, Browse all, Run as program on parents, one listing rule) (notes §8).
6. **Feature pane**: source header and the team-task board moved in (five columns); the **Board pane removed** and every entry point re-routed, one-time removal toast (spec §3.5, notes §1.3, §9).
7. **Write-back** (default on for GitHub / Linear): start label / status / comment / assign, team tasks as `orch` sub-issues of the source item, `Closes #n`, close on merge, program parent closure, failure state and retry (spec §3.6, notes §10.4, S15).
8. **Agents**: `read_issue` / `update_issue` take Linear identifiers and return `parent` / `children` / `dependencies`; a lead's first turn names its source item and its children (spec §3.7).

## 2. Exit criterion

The improve-intake exit criterion (its spec §2), checked here with the fake GitHub (`gh`) and fake Linear used by today's tests, end to end through the core: a GitHub epic with 3 sub-issues (one blocked by another) and a Linear project with 2 milestones are each started as a program with one plan approval; each feature starts only once its dependencies are merged; on merge the fake tracker shows the items closed / Done and the program parent closed; no Board pane exists and no agent pane was needed. Plus every board of notes §13 implemented and reviewed by ui against its export.

## 3. Work split (tasks after approval)

Each task owns its modules; one back and one front agent, so tasks of a node run in order. Front builds against [api-contract.md](api-contract.md) with mocked `invoke` (as `ui/test/*` do) and wires to the real commands as back's tasks merge.

| # | Node | Task | Owns | Depends on |
|---|---|---|---|---|
| B1 | back | **Tracker settings + migration**: `TrackerSettings` in the store (replacing `task_backend` + `intake_source`), migration with `migration_notice`, `tracker_settings[_set]`, `tracker_preview`, `tracker_options`, `tracker_notice_dismiss`; task sync and every reader of the two old settings switched to the tracker; old commands kept callable until I1 (contract §7) | `orch-store` (settings, migrations), `agent-runtime/src/intake.rs` settings half, `orch-app` commands | gate |
| B2 | back (opus) | **Tree readers + cache + refresh**: GitHub GraphQL and Linear tree readers, Local reader, `work_nodes` table replacing `intake_items` (and the remote half of `tasks`: task sync keeps only team tasks), one refresher with joined refreshes, per-project `SourceStatus`, Ready filter evaluation, `now.intake` from `work_nodes` (`node`, `breadcrumb`) | `orch-tasks` readers, `orch-store` work cache, `agent-runtime` refresher | B1 |
| B3 | back | **Work commands + app state**: `work_tree` / `work_search` / `work_node` / `work_reveal` / `work_refresh` / `work_hide` / `work_unhide` / `work_prefill` / `brief_prefill`, `AppState`, roll-up, `actions`, `run.source`, start links (`intake` link → node) | `agent-runtime` work module, `orch-app` commands | B2 |
| B4 | back (opus) | **Programs**: `program_plan` (grouping, edges, cycles, plan issues, room), `program_start` / `program_update` / `program`, the engine (dependency merges, max at once, Governor), `now.programs` deltas and lane, program decisions, `origin.program`, `decision.program`, events | `agent-runtime` program module, store `programs` tables, `now.rs` | B3 |
| B5 | back (opus) | **Write-back + agents**: start / PR / merge / parent writes, team tasks as sub-issues, `WorkNode.writeback`, `writeback_retry`, `writeback_failing` decision, `gh_write` / `linear_write`; MCP `read_issue` / `update_issue` additions; lead first-turn source item | `orch-tasks` writes, `agent-runtime` write-back, `orch-mcp` | B3 (B4 for program parent closure) |
| F1 | front | **Project settings › Tracker** (replaces Intake and Task backend sections; migration notices; errors; Advanced task states) | `ProjectSettingsWindow`, new `TrackerSettings.svelte`, removal of `IntakeSettings` / task-backend UI | gate |
| F2 | front | **Feature pane + Board removal**: source header, Tasks section (five columns, task moves, keys), landing from links, Board pane and its entry points removed / re-routed, removal toast, shortcut sheet and palette | `FeaturePane`, `BoardPane` (deleted), `RunInspector`, `WaitingChip`, chat links, `shortcuts.js` | F1 |
| F3 | front (opus) | **Work browser pane**: tree, rows, chips, glyphs, roll-up, fold, depth / Focus here, details, search, filters, view prefs, multi-select, actions, states, keys, entry points (sidebar, Project pane, palette, leader) | new `WorkPane` + `components/work/*`, `lib/work*.js` | F2 |
| F4 | front | **Mission control + start dialogs** (needs B3's `work_prefill`): Work button / `w`, Ready lane changes, prefilled dialogs additions (`work_prefill`, breadcrumb origin chip, sub-issues line, blocked line, write-back line) | `MissionPane`, `mission.js`, `ReadyRow`, `Dialogs` (launch / new-session), `intake*.js` | F3, B3 |
| F5 | front (opus) | **Programs UI** (needs B4): plan dialog, Programs lane + row + sub-rows, Program view pane, program cards and collapsed lines, spec / PR program tag and "next spec" focus | new `ProgramPlanDialog`, `ProgramPane`, `components/mission/ProgramRow`, chat cards | F4, B4 |
| U1 | ui | **Visual QA + README**: every implemented board checked against its export at 1512 and 1280, findings to front through lead; `docs/design/README.md` rows → Implemented | `docs/design/README.md` only | F5 |
| X1 | ux | **Copy, keys and states pass**: notes §8–§12 copy, keys (§12, conflict table) and states S1–S19 checked in the built app / tests; findings to lead | none (report only) | F5 |
| I1 | back | **Exit-criterion e2e** (§2) with fake gh / Linear, plus fixes it reveals in back's modules; removal of the old commands (contract §7) | tests, old commands | B4, B5, F5 |

Front reviews: each front task asks ui for a review of its screens against the exports before it reports done.

## 4. Out of scope

As improve-intake spec §4: other trackers, several trackers per project, cross-project programs, creating / editing tracker items from Work (beyond write-back), the Overwatch starting programs, auto-merge, skipping per-feature gates. Also, settled by the design: no board view in Work (F28), no batched spec approval (F29), non-sibling selection (F30).

## 5. Acceptance

1. `cargo test --workspace` and the UI tests (`bun run test`, lint) pass on the integration branch; new code has tests (store migration from each of the four §10.6 cases, tree readers against fake gh / Linear incl. paging, complexity and `gh_sub_issues` fallback, Ready filter, app state / roll-up / actions, plan grouping / edges / cycles / issues, engine ordering, write-back calls, UI components and keys).
2. The exit criterion (§2) passes as an automated test.
3. No surface mixes tracker items and team tasks; every start is a dialog; the Board pane and its commands are gone.
4. ui's visual QA and ux's pass report no open finding (or each is settled by lead in `decisions.md`).

## 6. Decisions for the user (proposed; approving accepts them)

| # | Question | Proposed |
|---|---|---|
| Q1 | The design is not in `main` yet | **Merge the improve-intake PR first**; lead then rebases this branch on `main` before creating code tasks |
| Q2 | Old commands (`intake_source*`, `intake_prefill`, `intake_item`, task backend settings, Board) | **Removed** in this feature, not kept as aliases (only the app calls them) |
| Q3 | One PR for the whole build | **Yes**, one PR at the end (tasks merge into the integration branch as they finish) |
