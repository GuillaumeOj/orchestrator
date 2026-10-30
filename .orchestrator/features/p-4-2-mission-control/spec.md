# P4.2 — Mission control · Feature spec

> Feature `p-4-2-mission-control` · Integration branch `orch/p-4-2-mission-control/integration` · Source: [docs/SPEC.md](../../../docs/SPEC.md) §11 P4.2 (v0.12), FR-1302 (replaces FR-901/903), J4, J12 · 5, §9.0 P4 gate, §9.2 "From P4.2", §9.3, §9.7 "From P4" · Why: [brief 2026-10-28](../../../docs/brief/2026-10-28-overwatch.md), [ADR-011](../../../docs/adr/011-overwatch-and-attention-model.md) §3
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (Penpot pass, copy, states) · [decisions.md](decisions.md)

## 1. Goal and exit criterion

SPEC §11: *"Penpot pass (§9.0 P4 gate), then Mission control replaces the Overview (FR-1302); sessions open over the board; the Inspector follows the selection."*

**Exit criterion: "What is running, waiting on what, queued why, done today" is answered on one screen.** Checked by (a) a UI test that renders Mission control from one `now` fixture and finds, without opening anything else, every running row with its activity, every wait with its reason and since, every queued agent with its reason, ETA and Run now cost, and every outcome of today; (b) the demo in §6.

P4.2 builds on P4.1's attention records (FR-1301), Decisions (FR-1303) and outcomes, which are already pushed over the `now` event. Mission control is a **render of `now`**: it fetches nothing and derives no state of its own (§9.1 principle 10).

## 2. What the code does today (HEAD bd5797c)

| Item | Today |
|---|---|
| Home pane | `OverviewPane.svelte` (FR-901/903): a table of agents grouped by project or state, filter chips from `counts()`, keys `j/k ⏎ i p s r ⇧K`. Opened by leader `o` and by default on a fresh layout (`restoreLayout`, app.svelte.js:746/773). |
| Attention | `app.now` (now.svelte.js) holds agent records, run records, counts (global and per project), decisions, outcomes. Run records: `run, feature, project, phase, phase_since_ms, level, members, blocker?, counts` (attention.rs:173). No lead id, no task progress, no PR link. Queued agents carry their Governor text in `reason` only; Run now's cost is computed by the UI from `app_state.agents[].queue_entry` (`runCost`, status.js). |
| Outcomes | `now_snapshot` returns today's outcomes with `seen`; the store keeps only unseen ones after a push (`now.svelte.js:97`), because only the Inbox digest uses them. |
| Opening a session | `openAgent` (app.svelte.js:446) splits right, or adds a tab when 3 panes are visible. An agent pane has the keyboard at once: every key goes to the PTY (§9.7), Esc included. |
| Inspector | `Inspector.svelte`: the focused pane's agent, else `app.selectedAgent`. Agents only. |
| Layout | Tiling tree of tab groups (`layout.js`), saved in settings `layout`; pane types include `overview`. |
| Penpot | P1–P3 screens Ready for dev; P4.1 components In review on page 02. The P4 gate (§9.0) is not drawn. |

## 3. Scope

### 3.1 Penpot pass — the P4 gate (§9.0) (M)
SPEC §9.0: *"the new shell (board + chat column), Mission control, every decision card kind with its lifecycle states, the collapsed Overwatch lines and the session overlay must be in Penpot (pages 03, 04, and a 06 storyboard of J12) before P4.2 UI work starts."* Two parts (decision D1):
- **Part A — blocks front**: page 03 new shell (Mission control centre, chat column on the right as drawn in §9.2 "From P4.2", Inspector docked/overlay, breakpoints 1280/1512/1728); page 04 Mission control (every lane, run row, solo row, queue row, outcome row, row details, project filter, empty states), the session over the board (watch and take-over), the Inspector for agent / run / decision; page 05 Mission control keys.
- **Part B — completes the gate, feeds P4.3**: page 04 the chat column with a decision card per kind and lifecycle state, collapsed Overwatch lines (expand, undo), rule proposal card, morning line, Overwatch status; page 06 the J12 storyboard.
Both parts are in this phase and both are reviewed by the user (`00 Cover` In review → Ready for dev). Front starts when Part A is handed off.

### 3.2 Mission control replaces the Overview — FR-1302 (M)
A new pane type `mission` (`MissionPane.svelte`); `OverviewPane.svelte` is deleted. It is the home pane: a fresh layout opens it, leader `o` and the palette ("Open Mission control") focus it, and a saved layout's `overview` pane is restored as `mission`.

**Lanes**, top to bottom, each with its header and a count of its **rows** with a unit ("Needs you · 1 run · 1 session", "Queued · 2 agents", "Done today · 4 outcomes"; D11); empty lanes are hidden except Needs you ("Nothing needs you"), from one pure function `lanes(now, filter)` in `ui/src/lib/mission.js` (unit-tested):

| Lane | Rows | Order |
|---|---|---|
| **Needs you** | runs and solo agents at level `needs_you`, then `error`; plus one line for open decisions with no agent ("2 decisions not tied to a session → Inbox") | oldest wait first |
| **Running** | runs and solo agents at level `running` | project, then oldest phase/activity first |
| **Waiting on** | runs and solo agents at level `waiting` (agent, task, limit, startup, external) | oldest wait first |
| **Queued** | every agent at level `queued` — the **whole** queue: Governor entries, not started, paused — one row per agent, run members included | queue position (paused and not started after the queue) |
| **Done today** | today's outcomes (task done, merge done, PR opened with its link, run phase, run done, solo turn done, report ready), seen or not | newest first; the last 10, then "Show all n" |

Below the lanes, a collapsed line **Idle · n sessions** (solo agents at `idle`, including suspended) that expands to rows (D3). Agents at `done` and runs in phase `done` are not rows; their outcomes are in Done today.

**A team run is one row** (FR-1302), in the lane of the run's `level` (D4): `project › feature`, the **phase stepper** (draft → spec → planning → executing → integrating → PR; `spec_gate` and `pr_gate` show the spec / PR step "at gate"; `paused`, `deferred`, `failed` mark the current step), phase since, task progress ("3/7 tasks done"), the PR link when open, the run's blocker line when it has one, and its **members as chips**, lead first, each with its status icon and its own short wait ("front-1 → back-1"). Queued members also appear as their own rows in Queued, so the lane lists the whole queue.

**A solo session is a row**: `project › session`, status badge, the waiting-on line (P4.1 `WaitingChip`: reason + since) or its activity when running.

**Queued rows** show the reason, the ETA and **Run now** / **Start now** with its cost in the window that binds it (FR-809), and Cancel — all from the record's `queue` field (api-contract §1), not from `app_state`.

**Row details** (`space` or a click on the row's disclosure): the FR-901 columns kept from the Overview, one line per agent of the row: activity, elapsed, ctx %, usage this session (pp), Governor, memory. The footer keeps the Overview's totals and memory line (counts from the core).

**Per-project filter** (from the first version): `f` or the toolbar opens a multi-select of projects (All, or one or several); remembered in settings key `mission` (`{projects: [ids] | null}`); rows follow the filter, so lane counts do too; the filter popover, footer and status bar show the core's per-project agent counts (D11). A filter that hides rows says so in the toolbar ("2 projects of 4").

**Empty states**: no projects (→ J0: Add project); nothing running (New session); everything waiting on you (Needs you only, with "Nothing else is moving").

### 3.3 Sessions open over the board (M)
- **⏎ on a row** (or a click on a solo row / member chip) opens the target agent's Claude Code pane **as a tab in Mission control's tab group** and shows it (D5). Shells stay in the dock. Other splits the user made are left alone.
- The session opens in **watch mode**: the terminal renders live but does not have the keyboard; a strip says "Watching back-1 · ⏎ take over · Esc back". **Esc** returns to Mission control (the tab stays open in the group); **⏎** or a click in the terminal **takes over**: the PTY gets the keyboard and every key goes to Claude Code (Esc included, §9.7). From take-over, leader `o` returns to Mission control.
- Pane interventions (FR-1311) are unchanged: only typed input counts, so watching never records one.

### 3.4 Keys (§9.7 "From P4")
Pane-local, when Mission control has focus: `j/k` move rows (across lanes, skipping headers; collapsed lanes skipped), `h/l` move between the member chips of a run row (the **target** agent), `⏎` open the target's session, `i` interrupt, `p` pause / resume, `s` suspend, `r` resume, `⇧K` kill (asks first), `space` row details, `d` open the row's decision (the Inbox on that decision until P4.3, the card after), `n` Run now / Start now on a queued row (confirms with the cost, FR-809), `f` project filter, `o` on an outcome opens it (PR link, feature, agent). A run row's default target is its blocker's agent, else its lead. Leader `o` opens Mission control. The full map goes on Penpot page 05.

### 3.5 The Inspector follows the selection (M)
`app.selection = {kind: 'agent' | 'run' | 'decision', id}` replaces `app.selectedAgent` (sidebar, palette and Mission control set it; the focused agent pane still wins, as today). Mission control sets it on every row move: run row → the run, or the target agent once `h/l` moved; solo row → the agent; Needs you line without agent → the decision; outcome → its run, else its agent.
- **Agent**: as today (status, waiting-on, details, actions).
- **Run**: feature and project, the phase stepper with since, members with status and reason, the blocker, tasks done / in progress / blocked, the PR link with its CI state when open (existing `PrLine`), actions Open feature, Open board.
- **Decision**: kind, title, subject, its context blocks (P4.1 `ContextBlocks`), lifecycle marker, Open in Inbox.

### 3.6 Core additions (S) — api-contract §1
- **Run record**: `lead` (agent id), members ordered lead first then by node order, `tasks {total, done, in_progress, review, blocked}`, `pr? {url, number?}` once `run.pr_url` is set, `started_ms`, and for paused / deferred / failed runs `held_in` and (deferred) `until_ms` (D12).
- **Agent record, queued**: `queue? {entry, position?, tier, eta_ms?, run_now {week_pp?, five_hour_pp?, binds}}`, present only when the agent has a Governor queue entry (statuses `queued`, `not_started`; never for paused) (D13). `position` is 1-based in the Governor's order.
- Both are pushed with the record (a change in progress or ETA is a record change). `orchd now` prints the task progress on run lines.

### 3.7 Done today keeps seen outcomes (S, front)
The store keeps today's outcomes after a push whether seen or not (the Inbox digest still filters on `!seen`); `counts.done_today` and the lane's list are the same set.

## 4. Out of scope (P4.2)
- The chat column, cards, collapsed lines, the Overwatch (P4.3/P4.4): designed in Part B, not built. The Inspector stays on the right; no empty chat column is shown.
- The Inbox pane stays until P4.3 (⌘⇧I, title badge). `d` and the Needs-you line lead to it.
- Intake and the *Ready to start* lane (P4.5).
- Team-level pause/resume from a run row (pause acts on the target agent; Pause all stays in the planner and title bar).
- Drag-and-drop between lanes, sorting options, grouping by project inside lanes.

## 5. Non-functional
- **One source of state**: Mission control reads `app.now` (and `app.data` only for an agent's raw details: ctx %, memory, model); it never calls a view command and has no timer except the 30 s relative-time clock.
- **Performance (NFR-1b)**: 50 agents + 10 runs render in < 16 ms per `now` event (test with a fixture); lanes are plain lists (no virtualization below 200 rows).
- **Accessibility**: rows are a `grid` with `aria-selected`; lane headers are headings; the phase stepper has a text equivalent ("phase 4 of 6: executing"); colour is never the only signal.
- **Tests**: `make ci` green. Core: run record `lead`, member order, task counts, `pr`; queued `queue` field (position, ETA, binding window) for a queue entry, a not-started agent, a 5h-guard hold. UI: `lanes()` table tests (every level, error in Needs you, idle line, done not a row, whole queue incl. run members, filter by one / several projects, lane counts = rows with their unit, empty lanes hidden except Needs you), keyboard (`j/k` across lanes, `h/l` chips, `⏎` opens a tab in Mission control's group, Esc in watch mode returns, keys reach the PTY after take-over), filter persisted and restored, a saved `overview` pane restored as `mission`, Inspector for run and decision, the exit-criterion test (§1). Tests hold at any date, hour and time zone.

## 6. Acceptance (demo at the PR gate)
1. With a team run (lead running, back-1 at a permission, front-1 waiting on back-1), a solo session running, one queued agent (live cap), one not-started agent, and outcomes today: Mission control shows the run in Needs you with its stepper at executing, its blocker "back-1 · permission: Bash …", its chips; the solo row in Running with its activity; both queued agents with reason, ETA and Run now cost; Done today with the PR link.
2. `f` → choose one project: other projects' rows and counts leave; restart the app: the filter is kept.
3. `j` to the run, `l` to front-1: the Inspector shows front-1; `h` back: the Inspector shows the run with tasks and members.
4. `⏎`: back-1's session opens over the board in watch mode; Esc returns; `⏎` again, `⏎` in the session takes over and a key reaches Claude Code; leader `o` returns.
5. `space` shows the row's agents with ctx %, usage, Governor and memory.
6. Approve the permission in the Inbox (`d`): the run row moves to Running without a refresh.
7. Penpot: pages 03, 04, 05, 06 with the P4 gate marked on `00 Cover`; exports under `design/`.
