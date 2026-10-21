# P1 design pass: notes for review

Design pass on 2026-10-20 for the P1 gate (ADR-008, SPEC §9.0). Penpot file **Orchestrator**, pages 02, 03 and 04. Everything below is a proposal for Guillaume to accept or reject. No SPEC edits were made.

## What was designed

- **02 Components** (13 component sets, states as variants, all fills, strokes, radii, spacing and text styles bound to semantic tokens):
  - Status badge: 9 states × {badge, icon}. Glyphs ▶ ◆ ◈ ⧗ ⊘ ○ ‖ ✕ ✓.
  - Button: {primary, secondary, ghost, danger} × {default, hover, focus, disabled}.
  - Icon button, key hint, agent chip (6 roles × {default, compact}), pressure pill (4 states).
  - Pacing indicator: {compact, expanded} × 4 pressure states.
  - Sidebar tree item: {project, feature, agent, solo} × {default, hover, selected, focus}.
  - Pane header: {focused, unfocused}.
  - Inbox item: {gate, permission, question, budget} × {default, selected}.
  - Queued explanation row: {budget, cap, paused}.
  - Banner: {limit reached, login expired, governor red}.
  - Toast: {undo, budget, info}.
- **03 Shell & layouts**: the shell at 1512 × 982 (title bar with pacing indicator and Inbox, sidebar tree, agent + nvim + zsh panes, docked inspector, status bar); the shell at 1280 × 800 (icon rail, overlay inspector); a zoomed pane; layout notes.
- **04 Screens** at 1512:
  - **Overview**: grouped by project, with live state, current tool, files, elapsed time, context, usage, governor status and memory.
  - **Agent pane**: front-1 waiting on a Claude Code permission prompt, plus a board with the queued, suspended, blocked-by-limit and error states.
  - **Week planner**: ideal vs actual vs forecast (at the current rate and with the governor), the 5h window with its guard, allocations with weights and tiers, next-action preview with a pending ask, and the governor log with undo.
  - **Inbox**: attention-ladder order with one gate expanded, a who/what/why preview, the rendered spec and the actions.
- **Tokens**: no token changes. `tokens.json` is unchanged.

## Review round 1 (Penpot comments 1–11), fixed

- Glyphs in pills and role letters are centered (fixed-size text boxes, centered both ways).
- **Shortcut convention:**
  - Every shortcut is one Key hint container.
  - "+" joins keys pressed together (⌘+K, ⌘+⇧+I).
  - A space separates the steps of a sequence (leader then key: ⌘+; g).
  - Pane headers, the status bar leader hint and the Agent pane strip now use key hints.
  - The key hint sits at the far right of buttons.
- **Sidebar tree:** indentation is now project 8 / feature and solo 24 / agent 48 (space.sm / xl / 3xl).
- **Count badge:** new component (height 16, about 16 min width, caps at 99+), used for the Inbox count and the feature rows.
- **Search icon:** drawn magnifier instead of the ⌕ glyph.
- **Icon rail:** the per-agent status glyphs are gone; it shows a count badge when agents need you, and hover opens a project summary popover (board "Rail hover · 1280").
- **1280 layout:** no pane hidden behind the overlay inspector.
- **Foundations:** the focus-ring example is no longer clipped.

SPEC §9.7 should adopt the shortcut notation above (proposal; not edited).

## Open questions for Guillaume

1. **Primary button style.** `text.inverse` on `accent.default` is 2.7:1 in Light, so primary buttons use `accent.subtle` fill + accent border + `text.primary`. OK, or should the Light accent be darkened so a filled primary works?
2. **Status icons in Light, and Suspended in both themes, are under 3:1 on `surface.raised`** (badge background). Suspended: 2.2:1 Dark, 2.0:1 Light. In Light, idle, queued, blocked, error, at gate and waiting-for-you are 2.3–2.95:1. The label always carries the meaning, but WCAG 1.4.11 wants 3:1 for the icon. Options:
   - (a) badges without a fill in Light (on `surface.pane` most reach 3:1, but Suspended stays at 2.5);
   - (b) darker `palette.day.*-aa` entries for the status colors;
   - (c) a new `color.status.suspended` value in both themes.
   My recommendation is (b) + (c).
3. **`text.muted` on `surface.raised`** is 4.25:1 in Light (README guarantees only pane and chrome). It appears in the overlay inspector field labels and the queued strip. Avoid muted on raised, or add raised to the contrast guarantee and adjust `day.muted-aa`?
4. **1728 breakpoint**: not drawn. Same as 1512 with a wider workspace, or a third pane column by default?
5. **Role marks**: roles use a letter in a role-colored outline (L, B, F, U, X, S), not an icon set. Is a letter enough as the role "icon" (SPEC §9.1.7)? A real icon set would need an icon font or SVGs in Penpot.
6. **IBM Plex Mono has no ⌘ ⏎ ⇧ glyphs**, so key hints use the UI font. In the app the mono font comes from kitty (FR-1002), so key hints should stay on the UI font there too.
7. **Inbox count in the title bar** counts levels 1–3 (4 here) and excludes the level-4 digest. Confirm.

## SPEC gaps found while designing (proposed edits, not made)

1. **ADR-007 / FR-803: the ideal curve vs the measured W.** `ideal(t)` excludes the reserve and the unattributed forecast, but `e = W − ideal` uses the total W, which includes reserve use and outside-app use. Early interactive use or claude.ai use then shows up as "over plan" and can push the Governor into Amber/Red, which squeezes autonomous work for usage the Governor doesn't control. In the scenario, agents are exactly on plan (38 %) but W = 47 % because of 5 pp reserve and 4 pp outside use.
   - Proposal: compute the error on the governed share, `e = (W − reserve_used − U_actual) − ideal(t)`, or add the reserve and U pro-rata to the ideal curve.
   - Also state which one the title bar shows ("plan 38 % (+9)").
2. **§9.5 has no "Paused" state.** FR-805 / FR-811 pause teams at a turn boundary, and "Pause all" exists, but the status vocabulary has only Queued and Suspended. I showed it as Queued with a "Paused by governor" reason row. Proposal: say in §9.5 that a governor pause is `queued` with reason `paused`, or add a 10th state.
3. **§9.3 Agent pane, queued state: what does the PTY show before the first turn?** For a queued agent that has never run, is `claude` started (idle, costing memory) or not started at all? I designed "not started, prompt ready". Proposal: state that queued agents with no session have no process (FR-901 memory then reads "—").
4. **J9: who owns the "blocked by limit" banner when only the 5h window is exhausted?** The banner copy differs (5h reset in hours vs weekly reset). Proposal: two banner variants, and J9 should say "a window" consistently.
5. **J5 / FR-707: the Inbox key map is not written down.** I used `j/k` move, `a` approve, `r` reject, `⏎` reply, `o` open, `x` select (batch), `u` undo, `e` open in nvim. Proposal: list it in §9.7 next to the leader map.
6. **J4 / FR-903: actions need keys in the Overview.** I used `⏎` open, `i` interrupt, `p` pause, `s` suspend, `r` resume, `k` kill (single keys while the Overview has focus). `r` clashes with Inbox "reject" only if both are focused at once, which can't happen, but write it down.
7. **§9.2 status bar "live/queued agents with the pressure state in words"**: the 5h guard has no place in the status bar. When the guard is on, the cap is 1 even in Green. Proposal: show "5h guard" next to the pressure pill when active.
8. **FR-809 Run now cost** is shown as "≈ +2 pp of the week". When the 5h guard is the binding constraint, the 5h cost matters more. Proposal: show the cost in the binding window ("≈ +6 % of 5h").
9. **§9.0 pages**: `00 Cover` lists screens "ready for dev". There is no "In review" status in the SPEC. The Cover uses Not started / In review / Ready for dev. Proposal: name these three statuses in §9.0.

## 06 Flows: J5 Inbox triage

Six 1512 frames on page 06, each with a notes board above it giving the step, the key sequence and what happens. A Penpot flow named "J5 · Inbox triage" starts at step 1.

1. **Gate selected** (`⌘+⇧+I`, `j/k`, `a`).
2. **Approved, permission next.** The undo toast offers `u`.
3. **Allowed, question next.**
4. **Reply composer.** `⏎` opens it, `⇧+⏎` adds a new line, `⏎` sends, `esc` cancels.
5. **Budget ask.** Shows its default and a timeout countdown (FR-810).
6. **Inbox clear.** Empty state; the Info digest keeps the history.

- **Prototype wiring:** Penpot has no keyboard triggers, so clicks stand in for keys: Approve → 2, Allow once → 3, Reply → 4, Send → 5, Defer → 6, Cancel → 3. The undo toasts go back one step.
- **Design decision:** after a gate is approved in Red, the lead **queues first for the live slot** instead of resuming, because back-1 holds the only slot. Interactive agents (front-1, solo/fix-tax) resume at once. The 04 Inbox screen now says the same.
- **Spec gap:** SPEC J5 step 3 says "after approval the agent resumes". Proposal: "resumes, or queues with its tier if the Governor holds the live slot (the item says so)".

## 06 Flows: J2 Solo session

Five frames with notes and the flow "J2 · Solo session":

1. **New session (⌘+T).** The dialog has project, role (plain Claude by default), task, name, base branch, a worktree toggle with a preview of the branch and folder, model and effort, and a Governor note.
2. **Session running.** The worktree is created, claude runs there, a zsh pane sits beside it, and the session appears under its project in the sidebar.
3. **Command palette (⌘+K).** Scoped to the focused agent: Create PR, Merge into main, Open diff.
4. **PR gate in the Inbox.** Title and body drafted by the agent, changed files, the local test run. On approval the app pushes and runs `gh pr create`; agents can't push (§6.5).
5. **PR opened.** The agent is Done, the strip links the PR and offers to remove the worktree, and the sidebar shows the PR number.

## 06 Flows: J8 Pacing the week

Four frames with notes and the flow "J8 · Pacing the week":

1. **Plan the week (Mon 09:02).** An Inbox Plan item prefilled from last week: calendar, weights, tiers, reserve, last week's outcome. If nobody answers, it applies at 09:20.
2. **Run now with its cost (Wed, Red).** A confirmation dialog shows the week and 5h impact, the cap it exceeds and how to undo it. It is logged as your override.
3. **Catch-up (Fri 16:00).** Usage is −12 pp with 2 working hours left. The live cap goes to 5, the queue is released, and the planner suggests the off-hours allowance for the weekend.
4. **Weekly report (Mon 09:00).** Used at reset, hours locked out, time in Red, governor actions, use per project, and suggestions for the next plan (none applied). "Plan week 44" loops back to step 1.

**Design decisions to confirm:**
- **Run now in Red runs above the cap** for that one turn instead of pausing back-1. SPEC FR-809 doesn't say which; proposal: "Run now admits above the cap for one turn and is logged as an override; it never pauses another agent".
- **The weekly report suggests plan changes** (reserve, ramp-up of team launches, weights) but applies none of them. FR-818 lists only usage and actions; proposal: add "suggestions for next week's plan (not applied)".
- **A week plan with no answer applies itself after 20 minutes,** like other asks (FR-810). FR-802 doesn't say; proposal: state it there.

## Not done
- **05 Keyboard & copy** is untouched.
- The pacing-indicator popover is a component, but no screen shows it opened over the title bar.
