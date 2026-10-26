# The app next to the Penpot design (first version)

Screenshots of the running app (release `.app`, 2026-10-26) next to the Penpot exports in
[`docs/design/img/`](../design/img). The app ran with real `claude --model haiku` agents (trivial
prompts) in a scratch home; the Penpot boards use the design scenario (Wednesday, Governor in Red,
a team on `checkout-v2`). So the **data differs** (this week, Green, solo sessions only); compare
layout, components, tokens and copy.

How they were taken: the UI was driven through the env-gated dev socket (`ORCH_DEV_CTL`, see
[crates/orch-app](../../crates/orch-app/README.md)), because computer control of the app was
declined; windows were captured with `screencapture -l`. Synthetic commands bypass AppKit, so
⌘-chords against the menu, dead keys and IME are **still not checked with physical keys** (R9).

| Board | Penpot | App |
|---|---|---|
| 03 Shell · 1512 (agent + nvim + zsh, inspector docked) | [03-shell-1512](../design/img/03-shell-1512.png) | [03-shell-1512](03-shell-1512.png) |
| 03 Shell · 1280 (icon rail, overlay inspector) | [03-shell-1280](../design/img/03-shell-1280.png) | [03-shell-1280](03-shell-1280.png) |
| 03 Zoomed pane | [03-shell-zoomed](../design/img/03-shell-zoomed.png) | [03-shell-zoomed](03-shell-zoomed.png) |
| 04 Agent pane (permission prompt) | [04-agent-pane](../design/img/04-agent-pane.png) | [04-agent-pane](04-agent-pane.png) |
| 04 Overview | [04-overview](../design/img/04-overview.png) | [04-overview](04-overview.png) |
| 04 Inbox | [04-inbox](../design/img/04-inbox.png) | [04-inbox](04-inbox.png) |
| 04 Inbox · Light | [04-inbox-light](../design/img/04-inbox-light.png) | [04-inbox-light](04-inbox-light.png) |
| 04 Week planner | [04-week-planner](../design/img/04-week-planner.png) | [04-week-planner](04-week-planner.png) |
| 06 J2 step 3 · command palette | [06-j2-3-palette](../design/img/06-j2-3-palette.png) | [06-palette](06-palette.png) |

What the screenshots show working end to end:
- **Agent pane**: the real Claude Code TUI in xterm.js (DOM renderer, pull transport), the kitty
  look (Menlo 11, the font kitty actually renders here; Tokyo Night Storm from kitty.conf).
- **Permission prompt answered from the Inbox**: WebFetch asked for permission; the
  `PermissionRequest` hook was held by the core, the item appeared in the Inbox and in the pane's
  strip, `a` approved it after the 5 s undo window, and Claude Code printed "Allowed by
  PermissionRequest hook".
- **Editor pane**: the user's nvim with its full config, opened through the editor template on the
  agent's worktree, listening on the project's socket. **Shell pane**: the login shell (zsh prompt,
  git, pager).
- **Recovery**: quitting the app suspends agents; the next start shows them Suspended with Resume.
  After a hard kill, the reconciler resumed the agent that was mid-turn ("ORCH: the orchestrator
  restarted while you were working…").

## Memory (NFR-1b, phys_footprint)

Release build, window 1512 × 982 at 1× on the external display, about 5–6 k free pages (8 GB
machine under pressure). "App" = the Rust core + the WebKit helpers the app is responsible for, as
the app's own memory figure counts it (FR-904). `claude` processes are listed apart.

| State | Core | WebContent | GPU | Networking | **App** | claude processes |
|---|---|---|---|---|---|---|
| 0 agents, Overview pane | 28 | 65–77 | 39–89 | 7 | **149–201** | — |
| 1 agent pane (+ Overview, Inbox tabs) | 29 | 120 | 86 | 7 | **242** | 240 (1) |
| 3 agent panes side by side | 30 | 133 | 84 | 7 | **254** | 667 (3: 211–233 each) |

- **NFR-1b holds** (≤ 250 MB idle; 254 MB with 3 busy panes is in line with P0's 6-pane figures).
- **Worse than P0's baseline (72–78 MB idle)**, almost all in the **GPU helper**: 28–41 MB with the
  shell hidden, 77–94 MB while it is visible and repainting. It does not depend on the window size,
  the web font or the window background (each was tested). The app repaints a little every second
  (status figures), which keeps WebKit's tile surfaces alive. Follow-up: poll less (push only
  changes) and check whether the IOSurface pool is purgeable.
- **WebContent** grows by ~45 MB with the first xterm pane (UI bundle + xterm.js + scrollback).
- One incident: a **runaway in WebContent (2.8 GB, 87 % CPU)** after shrinking the window to 1280
  and back while agent, nvim and shell panes were open. The process was killed at once (and with it
  the app; the reconciler later resumed the agent). The root cause was not isolated; the fit is now
  debounced with an A→B→A oscillation guard and programmatic focus no longer re-enters
  `focusPane`. The same sequence replayed afterwards under a watchdog stayed at ~106 MB. Keep an
  eye on it (it may be a resize/redraw feedback loop with Claude Code's full-screen reprint).

## Design deviations

1. **Features in the sidebar**: none yet (teams come in P2); agents are listed as solo sessions
   under their project. Role marks are derived from the agent name (solo → S).
2. **Pause per agent** (`⌘; p`, Overview `p`): the runtime has Pause all only; the app says so and
   offers Pause all. Paused shows when Pause all holds the agent's queued turn.
3. **Overview kill key** is `⇧K`: Penpot 05 gives `k` both to "move up" (j/k) and to "kill".
4. **Suspended resumes on ⏎** in its pane strip, not on focus (§9.3 "resume on focus"): resuming
   costs quota, so the app asks for one key.
5. **Inbox reply on a permission** = deny with the reply as Claude's message (option 3 of the
   CLI's dialog). A question (an agent waiting in its pane) is answered in the pane: `⏎` opens it.
   Penpot's "Allow once / don't ask again" choice is reached with "Answer in the pane".
6. **Info digest** lists finished turns (level 4); gates, budget asks and errors use the same item
   component. Gate items (spec, PR) arrive with P2.
7. **1280**: the overlay inspector covers the right pane when opened (Penpot avoided any covered
   pane by showing one pane only).
8. **Week planner** is read-only: Edit plan is disabled; allocation weights are bars, not sliders.
   Sprint mode (+5 pp), Pause all, undo and the pending ask work. The "actual" line starts when
   the app starts (no stored history yet); "w/ governor" is an estimate (the error closing
   linearly by the reset).
9. **Title bar**: native traffic lights over the bar (`titleBarStyle: Overlay`); the Inbox count
   badge uses the waiting-for-you color when non-zero and a neutral border at 0.
10. **Theme**: user choice (palette: Theme Dark/Light/system) > system. Deriving surface, text and
    accent from kitty (§9.1) is not implemented.
11. **Icons**: Material glyphs of FiraCode Nerd Font Mono loaded from the user's fonts through
    CoreText at start (not bundled in the `.app` yet).
12. **Diff / Open diff** and **Create PR** are not in P1's app yet (no diff pane).

## Spec edits (applied in SPEC v0.7, ADR-003, ADR-006)

| Edit | Where | In the code |
|---|---|---|
| Pane and Inbox both answer a permission prompt; the call's next hook closes the Inbox item | FR-705 | done (`orch-app`) |
| Overview kill is `⇧K` (`k` moves up) | §9.7 | done; Penpot page 05 still says `k` |
| ⌘⏎ submits a dialog | §9.7 | done |
| Suspended resumes on ⏎ or a click, not on focus | FR-204, §9.3 | done |
| Resizes debounced, A → B → A flips not applied again | FR-1003, ADR-006 | done |
| Measured app baseline with the full shell: 150–200 MB idle | NFR-1b | measured |
| Sibling worktree folder next to the main checkout, also for a project that is a linked worktree | FR-105, ADR-003 | **not yet**: `agent-runtime`'s `worktree` module still uses the registered folder's parent |
| Wrapped paths joined before matching | FR-208 | **not yet**: only unwrapped paths are links |
