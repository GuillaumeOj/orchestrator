# P4.4 — UX brief

> Companion of [spec.md](spec.md). The design file is FigCraft [`docs/design/Orchestrator.figcraft`](../../../docs/design/Orchestrator.figcraft) (Git LFS; [README](../../../docs/design/README.md), ADR-012). Start from the P4.4 parts already drawn: page **04.3** boards titled "(P4.4)" (card lifecycle "deciding", Part B collapsed lines with undo, rule proposal, Part B column states with Overwatch status and input), page **06.1 J12**, and [p4-2-design-notes.md](../../../docs/design/p4-2-design-notes.md) §8.1, §8.3–8.6, [p4-3-design-notes.md](../../../docs/design/p4-3-design-notes.md) §1 (the header slot and footer P4.4 replaces), §2 (line copy, `overwatch` row), §10 (keys). Exports of P4.3: `.orchestrator/features/p-4-3-chat-and-cards/design/p43-p44-*.png`.
> New notes go in `docs/design/p4-4-design-notes.md`. Exports go under `.orchestrator/features/p-4-4-the-overwatch/design/` (one PNG per board, prefix `p44-`).

## 1. Principles applied (SPEC §9.1)
- **Nothing waits on the Overwatch**: its status never disables a card or a key; a card it is deciding stays actionable.
- **Oversight**: everything it did is a line that says who, what and why (with its source), expandable, with undo when it still means something.
- **Every number says what it is**: "deciding · 42 s", "working · 2 events", "Asking again after 3 more".
- **Keyboard-first**: ⌘L the input, `/` search, `u` undo on a line.

## 2. ux — notes and copy (`p4-4-design-notes.md`), first
Settle each point, then hand off to ui (one note per point, with the copy):
1. **Header with the Overwatch** (spec §3.3, §3.9): title "Overwatch"; the status slot for `off · asleep · waking · working · paused · down` (P4.2 §8.6 copy, add `off` and `paused`); Restart when down; click on the status opens its session over the board; how the header keeps day / search / filter at 340 px.
2. **Input** in place of the P4.3 footer: placeholder, ⏎ / ⇧⏎, sending while asleep ("waking…"), while down / off ("Sent when the Overwatch is back"), the user's entry and the Overwatch's reply in the thread (author marks, markdown, source links, `reply_to`); what ⌘L and `/` do now.
3. **Deciding** (spec §3.9): the marker with its 60 s countdown, the hand-over to "Waiting for you", what changes when the user acts first; deciding is not in the badge.
4. **Annotation** with sources and a recommended action (highlighted, not pre-selected), on each card kind that gets one (permission user-only, gates, Governor ask, question it escalates); copy for "I'd allow it for this project" style notes.
5. **Overwatch lines** per verb (allowed, denied, answered, restarted, resumed, nudged, reprioritized, added) with `by_detail` and sources, and `· policy: <id>` replacing `· rule: <id>`; the core-denied line (`· core: outside the worktree`).
6. **Undo on lines** (spec §3.8 table): the label per line kind, the "Correct the answer" reply field, the undone state, "Can't be undone: …" copy, `u` on a focused line.
7. **Rule proposal** (spec §3.7): card with the rule, the target file, the three links; Add rule / Not now / Edit rule; added (5 s undo), not now ("Asking again after 3 more"), never again after two declines.
8. **Overwatch down / off / paused**: the column states, what a permission with effect `overwatch` shows when the Overwatch is down (straight to "Waiting for you").
9. **Settings** (page 04.7): the Overwatch section (on/off, model, batch window, permission seconds, idle grace, rule threshold) and Permissions (policy files with Open in editor, parse error and warnings, default effects for team roles and solo sessions, hold length).
10. **Week planner / report**: the Overwatch usage row (name, where it sits among roles).
11. **Keys** (page 05): ⌘L input, `/` search, `u` on lines, the input's own keys (Esc leaves it to the thread), conflicts with the card keys while the input has focus.
12. **J12** (page 06.1): replay the five frames with the built column (morning line, the day settled with lines, "why is checkout slow today?" answered, the third time a rule, watch don't open).
13. **States matrix** for the column with the Overwatch: asleep with no events today, working on 2 events, down with open cards, a user message pending, a long Overwatch answer, many Overwatch lines in a row (do they group? proposal: no grouping in P4.4).

## 3. ui — FigCraft, after ux hands off
- Page **04.3**: turn the "(P4.4)" boards into P4.4 boards per §2 (header + status, input states, deciding, annotation, lines with undo and undone, rule proposal states, down / off / paused, states matrix); the P4.3 footer board marked as replaced.
- Page **04.7**: Overwatch and Permissions settings.
- Page **04.6**: the Overwatch row in the planner / report.
- Page **05**: keys of §2.11.
- Page **06.1**: J12 frames.
- `00 Cover` and `docs/design/README.md`: P4.4 rows **In review**; tokens only if needed (`docs/design/tokens.json` and the CSS through the pipeline).
- Export every new or changed board to `design/` (`p44-*.png`), then hand off to front with the list of boards and the notes' sections. Review front's work on request.
