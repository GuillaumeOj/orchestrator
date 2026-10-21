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

## Open questions: answered 2026-10-21

1. **Primary button style:** kept (accent.subtle + accent border + text.primary).
2. **Status icon contrast:** done.
   - New palette entries:
     - Light: darker day `*-aa` entries for every status and pacing color.
     - Dark: storm `dark5-aa` (idle) and `red1-aa` (blocked by limit).
     - A neutral grey `suspended-aa` in both themes, so Suspended no longer looks like Idle.
   - Every status color is now ≥ 3:1 on `surface.raised` and `surface.selected` in both themes.
   - Suspended's glyph is now □; the previous ‖ is reused for the new Paused state.
3. **Muted text on raised surfaces:** `surface.raised` is now in the contrast guarantee, and `day.muted-aa` is #495178 (4.6:1 on raised).
4. **1728:** a wider workspace for now; revisit after using the app.
5. **Role marks:** the letter is fine for now.
6. **Nerd Fonts (your idea).** Proposal:
   - The app bundles *Symbols Nerd Font Mono*, a symbols-only font that sits beside the UI font and doesn't depend on the user's kitty font.
   - It would draw the key glyphs (`nf-md-apple_keyboard_command`, `…_return`, `…_shift`), the status icons and the role icons, so they render the same everywhere and line up on the cell grid.
   - Penpot can't load that font through the plugin API, so the file keeps Unicode stand-ins until the font is uploaded to the Penpot team.
   - Decision needed: adopt it for P1, or keep Unicode.
7. **Inbox badge:** counts levels 1–3, confirmed and added to SPEC §9.6.

## SPEC gaps: status 2026-10-21

The approved edits are now in SPEC v0.5 (§7 lifecycle, §9.0, §9.2, §9.3, §9.5, §9.6, §9.7, FR-809, J9) and in ADR-007 / ADR-008.

1. **Ideal curve vs measured W: open.** Today the error is `e = W − ideal`, where W includes your own interactive use and your claude.ai use. So the Governor can go Red because *you* used Claude, then squeeze the agents for it.
   - Example from the scenario: agents are exactly on plan (38 %), but W is 47 % because of 5 pp of reserve use and 4 pp outside the app, so e = +9 pp and the state is Red.
   - Option A (governed share): `e = (W − reserve_used − outside_actual) − ideal`. The Governor reacts only to agent usage. Your own heavy use is shown, but doesn't squeeze agents unless the week is really at risk (the exhaustion check stays on total W).
   - Option B (status quo): the Governor protects the week as a whole, so your own use slows the agents down.
   - Recommendation: A, with the exhaustion projection (`eta`) kept on total W so a lock-out is still prevented. The title bar would then show "plan 38 % (+0)" for agents, with a second number for the total.
2. **Paused:** added to §9.5 and §7 as its own state, teal, glyph ‖. The Governor's pause-team and your Pause / Pause all both use it.
3. **Not started yet:** no `claude` process until admitted. The queued state reads "Not started yet" and offers **Start now** with its cost (§7, §9.3, §9.5, FR-809).
4. **Limit banners:** one variant per window (weekly and 5h) in §9.3 and J9.
5. **Inbox keys:** already in FR-707; §9.7 now adds `e` and links to page 05.
6. **Overview keys:** in §9.7.
7. **5h guard chip:** in §9.2. It shows in the status bar whenever the guard holds work, in any pressure state.
8. **Cost in the binding window:** in FR-809 and ADR-007. Week costs are in **pp**, 5h costs in **% of 5h**, and the confirmation shows both.
9. **Page statuses:** Not started / In review / Ready for dev, named in §9.0.

## 06 Flows: storyboards

Prototype wiring was removed on 2026-10-21 (you said flows are not required, and Penpot can't trigger on keys). The frames stay as annotated storyboards. SPEC §9.0 and ADR-008 now say "storyboards".

### J5 Inbox triage

Six 1512 frames on page 06, each with a notes board above it giving the step, the key sequence and what happens. 

1. **Gate selected** (`⌘+⇧+I`, `j/k`, `a`).
2. **Approved, permission next.** The undo toast offers `u`.
3. **Allowed, question next.**
4. **Reply composer.** `⏎` opens it, `⇧+⏎` adds a new line, `⏎` sends, `esc` cancels.
5. **Budget ask.** Shows its default and a timeout countdown (FR-810).
6. **Inbox clear.** Empty state; the Info digest keeps the history.

- **Design decision:** after a gate is approved in Red, the lead **queues first for the live slot** instead of resuming, because back-1 holds the only slot. Interactive agents (front-1, solo/fix-tax) resume at once. The 04 Inbox screen now says the same.
- **Spec gap:** SPEC J5 step 3 says "after approval the agent resumes". Proposal: "resumes, or queues with its tier if the Governor holds the live slot (the item says so)".

### J2 Solo session

Five frames with notes:

1. **New session (⌘+T).** The dialog has project, role (plain Claude by default), task, name, base branch, a worktree toggle with a preview of the branch and folder, model and effort, and a Governor note.
2. **Session running.** The worktree is created, claude runs there, a zsh pane sits beside it, and the session appears under its project in the sidebar.
3. **Command palette (⌘+K).** Scoped to the focused agent: Create PR, Merge into main, Open diff.
4. **PR gate in the Inbox.** Title and body drafted by the agent, changed files, the local test run. On approval the app pushes and runs `gh pr create`; agents can't push (§6.5).
5. **PR opened.** The agent is Done, the strip links the PR and offers to remove the worktree, and the sidebar shows the PR number.

### J8 Pacing the week

Four frames with notes:

1. **Plan the week (Mon 09:02).** An Inbox Plan item prefilled from last week: calendar, weights, tiers, reserve, last week's outcome. If nobody answers, it applies at 09:20.
2. **Run now with its cost (Wed, Red).** A confirmation dialog shows the week and 5h impact, the cap it exceeds and how to undo it. It is logged as your override.
3. **Catch-up (Fri 16:00).** Usage is −12 pp with 2 working hours left. The live cap goes to 5, the queue is released, and the planner suggests the off-hours allowance for the weekend.
4. **Weekly report (Mon 09:00).** Used at reset, hours locked out, time in Red, governor actions, use per project, and suggestions for the next plan (none applied). "Plan week 44" loops back to step 1.

**Design decisions to confirm:**
- **Run now in Red runs above the cap** for that one turn instead of pausing back-1. SPEC FR-809 doesn't say which; proposal: "Run now admits above the cap for one turn and is logged as an override; it never pauses another agent".
- **The weekly report suggests plan changes** (reserve, ramp-up of team launches, weights) but applies none of them. FR-818 lists only usage and actions; proposal: add "suggestions for next week's plan (not applied)".
- **A week plan with no answer applies itself after 20 minutes,** like other asks (FR-810). FR-802 doesn't say; proposal: state it there.

## 05 Keyboard & copy

- **Shortcut maps:** global ⌘-chords, leader sequences and pane-local single keys (Inbox, Overview, Week planner, icon rail, agent pane).
- **P0 conflicts:** each with the decision taken.
- **Copy:** for the 8 global banners of §9.3, the macOS notifications per attention level, and the toasts.
- **Copy rules:** name the agent and project, use the §9.5 vocabulary, show usage in pp, give times with a relative hint, and say why and when for anything queued.
- **Proposal:** SPEC §9.7 should link to page 05 as the full key map. The pane-local keys (Inbox, Overview, planner) aren't in the SPEC yet.
- **Added:** I added a "5-hour limit reached" banner variant (spec gap 4 above). Version numbers in the "unsupported" banner are placeholders.

## Not done
- The pacing-indicator popover is a component, but no screen shows it opened over the title bar.
