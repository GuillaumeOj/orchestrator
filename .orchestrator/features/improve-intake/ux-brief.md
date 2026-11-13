# Improve Intake · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). The design file is the FigCraft file **in Google Drive** (`~/Library/CloudStorage/GoogleDrive-…/My Drive/Orchestrator.figcraft`, the project file attached to this project; ADR-012, [docs/design/README.md](../../../docs/design/README.md)). Do **not** commit `docs/design/Orchestrator.figcraft`.
> Start from: page **04.2** (Mission control, the P4.5 Ready to start lane and its rows), **04.4** (Feature, Board, Launch dialog, P4.5 prefilled dialogs), **04.7** (Project settings › Task backend / Linear, P4.5 › Intake), **02** (P4.5 components: Source glyph, Origin chip, Ready row), **05** (keys), and [p4-5-design-notes.md](../../../docs/design/p4-5-design-notes.md), [p4-2-design-notes.md](../../../docs/design/p4-2-design-notes.md).
> Notes go in `docs/design/improve-intake-design-notes.md`. Exports under `.orchestrator/features/improve-intake/design/` (one PNG per board, prefix `ii-`).

## 1. Principles
- **One kind of thing per surface**: tracker items in the Work browser and the Ready lane; team tasks only in the Feature pane; runs, sessions and programs in Mission control.
- **The tracker's words, the app's state**: every node shows what GitHub / Linear says (state, type, labels) and, distinct from it, what the app is doing with it (running, PR open, queued in a program). The two never share a colour.
- **Starting stays yours**: every start is a dialog the user submits; a program is started by one approval of its plan, then runs by itself.
- **Hierarchy is the point**: depth, parents and roll-ups must read at a glance and stay usable at 6+ levels and 500+ items.
- **Keyboard-first**, as Mission control.

## 2. ux — notes first (`improve-intake-design-notes.md`), then hand off to ui
Settle each with copy, states and keys; add a "Findings for lead" table (contract gaps, numbered F1…).
1. **Information architecture**: the three contexts (Mission control, project, feature) after the change; where the Work browser opens from (Mission control header + key, Project pane, sidebar, palette); what replaces each Board entry point (spec §3.5); the Board's removal.
2. **Work browser**: pane anatomy (tree / outline + details panel; scope = all projects or chosen ones, with a project level on top when several); the tree row (glyph per kind: GitHub issue / issue type, milestone, Linear project / milestone, ref, title, upstream state, labels ≤ 2, children progress, app-state chip, dependency mark "blocked by #9"); indentation and depth beyond 4; collapsed "Team tasks" fold; roll-up on parents; optional board (by upstream status) view — decide whether it earns its place.
3. **Details panel**: rendered body, breadcrumb, children list, dependencies (both ways), linked PRs, app history ("started 12 Nov · feature checkout-address · PR #61 merged"), estimate per mode, actions.
4. **Search and filters**: `/` search with results as breadcrumbed rows; filters (state, label, assignee, milestone / cycle, type / Linear project, only ready, only not started, hide closed); saved per scope.
5. **Actions and dialogs**: Start solo / Start team run (P4.5 prefilled dialogs, now with the breadcrumb and the sub-issues listed in the brief); Run as program; multi-select; Open in GitHub / Linear; Hide from Ready.
6. **Program plan dialog**: grouping (per child / grand-child / single), the planned features (list with dependency arrows or a small graph: choose), per feature mode / team / skip / merge, plan issues (cycle, already running, closed, no team, over room) and how each is fixed, total estimate vs the week's room, max at once, sequential toggle, Start program.
7. **Programs in Mission control and the Program view**: the program row (title, source, counts, sub-rows per feature with "waiting for <slug> to merge"), its place among lanes, the Program view (live plan, pause / resume / cancel / skip / add new upstream item), the cards it raises (new child upstream, child closed upstream, dependency added, program done), the batched spec-approval card if you recommend it.
8. **Ready to start lane** changes: breadcrumb on rows, **Browse all**, program-queued items.
9. **Feature pane**: source item header (breadcrumb, link) and the team-task board moved in (assign, status, edit, open upstream); how a sync conflict / `T-n` link lands there.
10. **Project settings › Tracker**: Local / GitHub / Linear; per kind fields (repo, Projects v2 board; Linear teams and projects); the Ready filter; write-back block (on / off, assign me, running label, what it writes, in plain words); refresh; Preview; the **migration notice** (two settings merged; a mismatch to resolve); errors per code (incl. `linear_complexity`, `gh_scope`).
11. **States**: first load (no cache), refreshing, offline (cached, "updated 2 h ago"), source errors, rate limited with retry time, empty tracker, no match for filters, huge tree (500+ roots, 100 children), closed parents with open children, item deleted upstream while running, write-back failed.
12. **Keys** (page 05) from api-contract §5, conflict check against Mission control and agent rows.

## 3. ui — FigCraft, after ux hands off
- **04.2**: Mission control with the Work entry, program rows, Ready lane changes (1512 and 1280).
- New page **04.8 Work** (or rows on 04.4; ui decides, README updated): Work browser at 1512 (GitHub fixture: epic → sub-issues → sub-sub-issues with types and a dependency; Linear fixture: project → milestones → issues → sub-issues), 1280, details panel, search, filters, every state of §2.11, multi-select.
- **04.4**: Program plan dialog (+ issues states), Program view, prefilled dialogs with breadcrumb, Feature pane with team tasks and source header; the Board boards marked "Removed in improve-intake".
- **04.3**: program cards (new child, closed upstream, dependency added, done, batched spec approval if kept).
- **04.7**: Project settings › Tracker (each kind, Ready filter, write-back, preview, migration notice, errors).
- **02**: components (tree row and its states, app-state chip, kind glyphs, program row). **05**: keys. **00 Cover** and `docs/design/README.md`: rows **In review**. Tokens only if needed (through `docs/design/tokens.json`).
- Export every new or changed board to `design/` (`ii-*.png`), commit the exports and the README, then report to lead with the list of boards.
