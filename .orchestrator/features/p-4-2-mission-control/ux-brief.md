# P4.2 — UX brief

> Companion of [spec.md](spec.md). P4.2 is the **P4 Penpot gate** (SPEC §9.0) plus Mission control in code. Penpot file: see [docs/design/README.md](../../../docs/design/README.md). Reuse the P4.1 components (page 02 "P4.1 · …": palettes, waiting-on chip, context blocks, lifecycle marker, outcome row, count labels) and the copy in [p4-design-notes.md](../../../docs/design/p4-design-notes.md); add P4.2 notes as a new part of that file (or `p4-2-design-notes.md` if it grows past ~200 lines).

## 1. Principles applied (SPEC §9.1)
- **Nothing requires a pane** (9): every row says what it waits on and since when; opening a session is for watching.
- **One source of state** (10): rows show the record's `status`, `reason`, `since`, the run's `phase`; lane counts are the core's counts (summed over the filtered projects).
- **Every number says what it is** (11): "Needs you · 2", "3/7 tasks done", "executing · 2 h", "Run now ≈ +2 pp week", "≈14:20", "2 projects of 4".
- **Keyboard-first** (1): every row action has a key (spec §3.4); shown as key hints in the toolbar.

## 2. Part A — blocks front (pages 03, 04, 05)

### 2.1 Shell (page 03), breakpoints 1280 / 1512 / 1728
SPEC §9.2 "From P4.2": title bar, sidebar (or icon rail < 1512), **Mission control** as the centre, the **chat column** on the right (drawn now, built in P4.3), Inspector, status bar, shell dock. Decide and annotate: at 1512 where the Inspector goes when the chat column exists (proposal: Inspector overlays the chat column, ⌘I toggles; docked only ≥ 1728). In P4.2 code the right column is the Inspector only.

### 2.2 Mission control (page 04)
- **Lane** header: name, count with its unit, collapse chevron; Done today collapsed after 10 rows ("Show all 14").
- **Run row**: `project › feature`, phase stepper (6 steps; at-gate, paused, deferred, failed marks on the current step), phase since, "3/7 tasks done", PR link `PR #12 ↗` with CI dot, blocker line, member chips (role colour + icon + name + status icon + short wait), selected chip state (`h/l`).
- **Solo row**: `project › session`, status badge, waiting-on chip or activity, since.
- **Queue row**: agent chip, reason, ETA, position ("2nd in line"), tier, **Run now ≈ +2 pp week** / **Start now**, Cancel.
- **Needs-you row** variants: permission, question, gate (spec, PR), Governor ask, error (error badge, after the blocking ones); the line for decisions with no session.
- **Outcome row** (P4.1 component) in Done today, with the PR link; seen and unseen look the same here.
- **Idle line**: "Idle · 3 sessions" collapsed and expanded.
- **Row details** (`space`): per agent: activity, elapsed, ctx %, usage (pp), Governor, memory.
- **Toolbar**: project filter (All / multi-select popover with counts per project), key hints. **Footer**: totals and memory (Overview's).
- **States**: normal (fixture §4), no projects (→ J0), nothing running, everything waiting on you, a filter that hides rows, 30 agents (density check at 1280).

### 2.3 Session over the board (page 04)
The agent pane as a tab in Mission control's group: **watch mode** (strip "Watching back-1 · ⏎ take over · Esc back", terminal dimmed focus ring), **take-over** (normal agent pane header, hint "⌘+; o Mission control"). Tab bar shows Mission control first.

### 2.4 Inspector (page 04)
Three variants: agent (P1 inspector with the P4.1 waiting-on line), **run** (stepper, members with status and reason, blocker, tasks, PR, Open feature / Open board), **decision** (kind, title, subject, context blocks, lifecycle marker, Open in Inbox).

### 2.5 Keys (page 05)
Mission control's pane-local map (spec §3.4) and leader `o`; check conflicts with the Inbox map (`d`, `n`, `f`, `space`) and with `k` vs `⇧K`.

### 2.6 Copy (UX)
Lane names and counts, row lines, stepper labels (draft, spec, planning, executing, integrating, PR; at gate, paused, deferred, failed), watch-mode strip, empty states, filter labels, the no-session decisions line, Run now confirmation (both windows, FR-809), toasts for row actions (only after the core confirms: P4.1 rule).

## 3. Part B — completes the gate, feeds P4.3 (pages 04, 06)
- **Chat column** (page 04): a **decision card per kind** (permission, question incl. multi-question, gate spec / PR / restart, startup, ask, budget, trust, wait, error/alert, notion) built from the P4.1 context blocks, with **every lifecycle state** (open, deciding by the Overwatch, sent, effect, failed, parked, stale) and the Overwatch's annotation; **collapsed decision line** (collapsed, expanded, with undo, undo expired); **rule proposal card**; **morning line**; **outcome digest** entry; **chat input**; **Overwatch status** (asleep, waking, working, down — cards still work); threading under a run (follow-up under the earlier answer); many open cards.
- **J12 storyboard** (page 06): the 5 steps of SPEC J12, annotated frames with the key sequence of each step.

## 4. Fixture for the frames and the exit test
Projects shop-api, blog, infra. Run `checkout` (shop-api) executing, 3/7 tasks, lead-1 running, back-1 at a permission (Bash pnpm add zod, 2 min), front-1 → back-1 · API contract · 12 min. Solo `blog › fix-feed` running "Edit src/feed.ts". `infra/deps` queued "live cap 2/2 · 1st in line · ≈14:20", Run now ≈ +2 pp week. ui-1 (checkout) not started. Done today: T-3 done, back-1 merged into integration, `blog #12` PR opened ↗, checkout planning → executing.

## 5. Who does what
- `ux` → copy and states (§2.6, §2.2 states, §2.5 conflicts), the lane and keyboard rules as annotations, J12 storyboard content; notes in `docs/design/`. Hands off to `ui`.
- `ui` → Part A in Penpot (pages 03, 04, 05), `00 Cover` In review, exports under `.orchestrator/features/p-4-2-mission-control/design/` (one PNG per board), new tokens in `docs/design/tokens.json`, README rows; hands off to `front`; then Part B (pages 04, 06). Reviews front's Mission control on request.
