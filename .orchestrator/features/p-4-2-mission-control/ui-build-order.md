# P4.2 Part A: Penpot build order (ui, T-59)

How the Part A boards get drawn, in order, from [p4-2-design-notes.md](../../../docs/design/p4-2-design-notes.md) (**N §n** below) with lead's decisions D11–D13. Each step lists what it reuses, what it adds and the PNG it exports to `design/`. Fixture: ux-brief §4, Thu 29 Oct 14:00, Dark theme.

## 0. Before drawing
1. Penpot tokens: mirror the tokens added to `docs/design/tokens.json` (commit 344408c) in the Penpot sets: `core` gets `size.chat` 360, `size.centre-min` 640, `size.lane-header` 32, `size.row-dense` 40, `size.chip` → `{size.control-sm}`, `size.strip` → `{size.statusbar}`, `size.stepper-step` → `{size.status-dot}` (8, the drawn dot), `size.stepper-link` 12. `theme/dark` and `theme/light` get `color.stepper.done` → `{color.text.secondary}`, `.todo` → `{color.text.muted}` (≥ 3:1 on raised and selected), `.link` → `{color.border.strong}`, `.mark` → `{color.text.primary}`. The current step uses the existing `color.phase.*` entry.
2. Read the 02 P4.1 sections (palettes, waiting-on chip, context blocks, lifecycle marker, outcome row, count labels) and the 03 / 04 P1 boards (title bar, sidebar, rail, status bar, Inspector, agent pane, Overview) to get their component ids. New boards are built from instances, never redrawn.

## 1. Page 02: new components, section "P4.2 · …" after the P4.1 sections
Each component set has its states as variants. Order: the small parts first, since the rows contain them.

| # | Component | Variants (property = values) | Source |
|---|---|---|---|
| 1 | **Phase stepper** | Step = Draft…PR (current step) · Mark = none, at gate, paused, deferred, failed. Steps before current filled `stepper.done`, current `phase.*` with its glyph, after hollow `stepper.todo`, joined by 12 px links (`stepper.done` up to the current step, `stepper.link` after). Each label sits right of its dot (compact: dots only + the current label). The text after it ("executing · 2 h") belongs to the row, not the stepper. | N §1.2 |
| 2 | **Member chip** | Status = working, permission, agent wait, task wait, limit, queued, not started, paused, error, idle, suspended, done · Selected = no, yes · Busy = no, yes (action pending, N §2 #34). Role color bar + status glyph + name + short wait ≤ 32 chars. Plus a "+3 more" chip. | N §1.3 |
| 3 | **Lane header** | Lane = Needs you, Running, Waiting on, Queued, Done today, Idle · State = open, collapsed, empty (Needs you only: "Nothing needs you"). Counts come from the 02 P4.1 count labels ("Needs you · 1 run · 1 session"). | N §1.1, D11 |
| 4 | **Run row** | Kind = blocker, running, gate, paused, deferred, failed (error), no PR / no tasks · Selected = no, row, chip · Details = closed, open. Line 1: title, stepper, phase text. Line 2: tasks, PR link + CI dot, blocker / error line. Line 3: chips. | N §1.3, §2 #2–9 |
| 5 | **Solo row** | Status = working, waiting for you, waiting on (agent / limit / task / external), error, idle, suspended · Selected. Status badge (P1) + waiting-on chip (P4.1). | N §1.4 |
| 6 | **Queue row** | Kind = queued, not started, paused, no cost, run member, 5h hold · Tier tag = none, low, high, critical · Selected. Buttons Run now / Start now / Resume + Cancel. | N §1.5, §2 #12–17 |
| 7 | **Needs-you line** | the no-session decisions line (one and two decisions). | N §1.1 |
| 8 | **Row details** | one table, 7 columns (N §1.8), queued row with "—". | N §1.8 |
| 9 | **Strip** | Mode = watching, watching paused, stopped, typing (take-over), typing stopped. 24 px. | N §1.10 |

The outcome row is the P4.1 component (no new component); its PR variant gets the "#12 ↗" link if it lacks one.

Export one PNG per section: `p42-phase-stepper.png`, `p42-member-chip.png`, `p42-lane-header.png`, `p42-run-row.png` (run row + row details), `p42-solo-row.png`, `p42-queue-row.png`, `p42-strip.png` (strip + Needs-you line).

## 2. Page 03: the new shell (row at y = 3300, after the P1 boards)

| # | Board | Layout (N §4) |
|---|---|---|
| 1 | **Shell · 1512 · Mission control (P4.2 code)** 1512 × 982 | sidebar 240 · Mission control · Inspector docked 320 (D11: docked ≥ 1512 in P4.2) · status bar · shell dock collapsed. This is the frame front builds. |
| 2 | **Shell · 1512 · with chat column** 1512 × 982 | sidebar 240 · centre 912 · chat 360 (placeholder, Part B content) · Inspector as an overlay in the chat's slot (open state). Annotation: rules 1–5 of N §4. |
| 3 | **Shell · 1728 · with chat column** 1728 × 1117 | sidebar 240 · centre 808 · chat 360 · Inspector docked 320. Annotation: "docked ≥ 1728" (SPEC §9.2 edit in P4.3). |
| 4 | **Shell · 1280 · rail** 1280 × 800 | rail 48 · centre · Inspector overlay (P4.2 code); chat variant as an inset (centre 884 · chat 340). |
| 5 | **Shell · shell dock open** at 1512 | dock below Mission control, sessions not in the dock (N §6). |
| 6 | **Layout notes · P4.2** | the width table of N §4 and the docking rule. |

Exports: `p42-shell-dock.png` (1512 P4.2 code with the shell dock open; the dock-collapsed frame is `p42-mission-control.png`), `p42-shell-1512-chat.png`, `p42-shell-1728.png`, `p42-shell-1280.png`, `p42-layout-notes.png`.

## 3. Page 04.2 Mission control (was page 04 at y = 9000 before the split)

| # | Board | Content |
|---|---|---|
| 1 | **Mission control · 1512** | the fixture: Needs you · 1 run (checkout, back-1 chip selected) + no-session line; Running · 1 session (blog › fix-feed); Queued · 2 agents (infra-1, ui-1 · shop-api › checkout); Done today · 4 outcomes; toolbar "All projects" + hints; footer. Inspector = run. (N §2 #1–3, #21, #24, #27) |
| 2 | **Mission control · row types** | every run, solo, queue and Needs-you variant from the 02 sets, stacked in lanes, plus Idle collapsed / expanded and Done today "Show all 14". (#4–23) |
| 3 | **Mission control · filter** | popover open (counts per project, "Remembered for next time"), and the toolbar "2 projects of 4" accent with its tooltip. (#25–26) |
| 4 | **Mission control · row details** | the run row with `space` open. (#9) |
| 5 | **Mission control · states** | no projects, nothing running, everything waits on you, filter hides all, loading skeleton, action pending / failed toast. (#28–31, #34–36) |
| 6 | **Mission control · 1280 · 30 agents** | density: 40 px rows, chips wrap after 4, footer pinned. (#32) |
| 7 | **Session · watch mode · 1512** | tab bar Mission control first + back-1; dimmed terminal, focus ring muted, strip "Watching back-1 · ⏎ take over · Esc back". |
| 8 | **Session · take-over · 1512** | normal agent pane, strip "Typing in back-1 · ⌘+; o Mission control"; inset of the stopped strips. |
| 9 | **Inspector · agent / run / decision** | three Inspector columns side by side (N §5.1–5.3), with the decision's context blocks and lifecycle marker. |

Exports: `p42-mission-control.png`, `p42-mission-rows.png`, `p42-mission-filter.png`, `p42-mission-details.png`, `p42-mission-states.png`, `p42-mission-1280.png`, `p42-session-watch.png`, `p42-session-takeover.png`, `p42-inspector.png`.

## 4. Page 05: keys (section after the P2 maps)
One board **Mission control keys**: the table of N §3.1, leader `⌘+; o`, the conflict results of N §3.2, and the note "`r` = Resume in Mission control, Reject in the Inbox". Export `p42-keys.png`.

## 5. Close
`00 Cover`: page index and changelog line "P4.2 Part A · In review · 2026-10-29". Export every board as PNG (Dark), then commit, hand off each PNG to `front` with the Penpot board link in the note, and report done.

---

# Part B (T-60): build order

Copy: notes §7 (J12) and §8 (cards), P4.1 §3 (kind labels, titles, blocks, markers) and §4 (effects). Cards are 336 px wide inside the 360 px chat column. Exports are prefixed `p42b-`.

| # | Page · board | Content | Export |
|---|---|---|---|
| 1 | 02 · "P4.2 · Decision card (Part B)" | Card anatomy: header (decision-kind glyph + label in `color.decision.*`, age), title, subject, P4.1 context blocks, Overwatch annotation (muted, prefix "Overwatch:"), actions with keys, P4.1 lifecycle marker. Open cards have a `attention.needs-you` outline; the others `border.subtle`. | `p42b-card-anatomy.png` |
| 2 | 04.3 Chat column · "Chat · cards per kind" | Permission, question (1 and n), spec gate, PR gate, restart gate, startup, ask, budget, trust, wait, error / alert, notion (§8.2). | `p42b-cards-kinds.png` |
| 3 | 04.3 · "Chat · card lifecycle" | One permission card per state: open, deciding by the Overwatch, parked, sent (undo), effect, failed, stale, dismissed (§8.3). | `p42b-cards-lifecycle.png` |
| 4 | 04.3 · "Chat · collapsed lines" | Collapsed, expanded, with undo, undone, undo expired (§8.4). | `p42b-collapsed-lines.png` |
| 5 | 04.3 · "Chat · rule proposal" | Open, added (undo 5 s), declined (§8.5). | `p42b-rule-card.png` |
| 6 | 04.3 · "Chat · column states" | Morning line, outcome digest entry, chat input (normal, asleep → waking, down), Overwatch status (asleep, waking, working, down), threading under a run, many open cards ("3 more open · j k"). Full 360 × 982 column. | `p42b-chat-states.png` |
| 7 | 06 (y = 9000) · "J12 · 1–5" | Five 1512 × 982 frames: chat column + Mission control, numbered callouts, the key line, the footer "Interventions this day: 0" (§7). | `p42b-j12-1-morning.png` … `p42b-j12-5-watch.png` |
| 8 | 00 Cover | P4 gate Part B In review, changelog 1.5. | — |
