# Flows list (R15)

> Round 2, T-295. For ui's T-296: redraw page **06 Flows** for the new UX, in the "Improve UI UX" row at the top of the page; old flows move below, tagged "superseded". Each flow is one row of 4–7 small frames (arrows, a caption under each, the key in the caption only, never as a pill in a frame). The frames reuse the 03/04 screens (instances, not redraws) and mark the changed element with an accent outline and a numbered dot. Companion of [spec.md](spec.md) §7, [ux-brief.md](ux-brief.md), [design-notes.md](design-notes.md).
> Exports: `design/06-flow-<id>.png`, one per flow (the whole row).

Common rules: single pane, project context Select, dock Agents/Shells, floating windows, `⋯` menus (pane headers: identity left, icon actions, `⋯` last). Sample project `orch`, second project `fusily`; feature `feat-x`; agents `lead`, `back-1`, `ui-1`.

| # | Id | Flow | Steps (one frame each) | Rules it shows |
|---|---|---|---|---|
| F1 | `start` | First launch to first agent | 1 Setup pane (app-wide) · 2 add project (dialog) · 3 context becomes the project, empty workspace with **Open orch / New session** · 4 New session dialog · 5 agent appears in dock Agents (waiting dot gone) · 6 agent pane | Empty state per context; opening an agent switches the dock to Agents |
| F2 | `context` | Work in one project | 1 All projects: Mission control · 2 click project `orch` in the sidebar → context `orch`, tabs filtered · 3 open feature `feat-x` (new tab, full title) · 4 switch context to `fusily` with the Select (its first tab activates; Mission control stays active when it was) · 5 open a `fusily` pane from All projects (context stays All) | Context filters tabs; All keeps All; app-wide panes always shown |
| F3 | `tabs` | Many tabs | 1 12 tabs overflow at 1280 · 2 ‹ › scroll, ends disabled · 3 open a tab off-screen: it scrolls in · 4 ⌘3 = third tab of the context · 5 ⌘W closes, ⌘⇧T reopens | R6 fade and buttons; tabs never shrink |
| F4 | `dock` | Agents and shells in the dock | 1 Agents view, All projects · 2 filter to `orch` · 3 an agent of another project needs you: dot on the project Select only (not on the view Select) · 4 switch to Shells: two shells, live titles, dot now on the view Select (an agent waits)  · 5 `cd` in a shell: title follows (`zsh · ~/dev/orch/ui`) · 6 empty Shells: inline New shell | View/filter Selects; waiting dot; live title |
| F5 | `agent-state` | A suspended agent, one home per action | 1 banner shows only app-wide conditions (none) · 2 sidebar badge on the agent · 3 agent pane strip **Resume** (the only place) · 4 header `⋯` open, pressed trigger, menu anchored under it (R7) · 5 agent waiting for you: strip answers, Inspector has details only | Dedup rules 1–3; R7 |
| F6 | `decision` | Answer a decision | 1 title bar Decisions (count · label · icon, one gap token: R5) · 2 Overwatch column, resizable handle (R1) · 3 decision card: Allow · 4 agent resumes, status dot changes · 5 policy "Always allow" confirm | Chat column handle; one home per action |
| F7 | `feature` | A feature from plan to PR | 1 feature pane running: icon buttons Pause, Stop, Delete + `⋯` (R9) · 2 doc tabs with the repo path line and Open in nvim icon (R10) · 3 review-done · 4 PR open: PR line, CI dot, checks popover (R13) · 5 `⋯` › Publish to Notion · 6 Board / Canvas from the `⋯` | R9, R10, R13 |
| F8 | `project` | Project pane actions | 1 header: gear, bin (danger), `⋯` · 2 `⋯` open: Fetch, Shell, Diff main, Trust · 3 worktree row: Diff ± icon, Remove… bin icon (R2, R3), tooltips · 4 remove worktree confirm · 5 gear → Project settings window | R2–R4 |
| F9 | `settings` | App settings window | 1 gear/⌘, over the app, dimmed backdrop · 2 section Integrations, a card needs you · 3 deep link from a banner lands on Claude Code · 4 edit a key, Save · 5 Esc closes, focus returns | Floating window; deep link; no Kbd in nav |
| F10 | `project-settings` | Project settings window | 1 project pane gear · 2 General (rename, default branch, Save) · 3 Task backend: Linear not connected → link opens Settings › Integrations (replaces the window) · 4 Files: access Select · 5 close | One window at a time; Select height (R11) |
| F11 | `planner` | Week planner | 1 title-bar pacing indicator · 2 Week planner window (1100 wide) · 3 plan a feature into a slot · 4 close; status bar has no planner entry | Status bar is information only |
| F12 | `shortcuts` | Find a key | 1 hover a button: tooltip `Resume · ⏎` · 2 ⌘/ opens the Keyboard shortcuts window · 3 search "dock" · 4 palette ⌘K: bindings right-aligned · 5 empty pane's one-line hint | Keys live in tooltip, palette, sheet |
| F13 | `terminal` | Shell and editor | 1 New shell in the dock (cwd = focused worktree) · 2 open a file from agent output → nvim tab (no split) · 3 Terminal & editor settings: theme, renderer, editor command · 4 Open in kitty | No splits; settings sections (R14) |
| F14 | `migrate` | Restart and old layouts | 1 quit with context `orch`, dock Shells, tree partly collapsed · 2 relaunch: same `workspace.project`, `dock.view`, `dock.agentProject`, `dock.activeAgent`/`dock.activeShell` and `sidebarTree`; an `activeAgent` outside the filter falls back to the filter's first agent tab · 3 a saved split layout restores as tabs in reading order | Persistence keys; layout migration |

## Frame inventory per flow

Use existing frames where one exists (03-*, 04-*); draw only what is missing: F1 steps 1–4, F2 steps 2 and 4, F4 steps 3–4 (new dot rule) and 5, F6 steps 3–5, F7 step 5, F8 steps 2 and 4, F9 step 4, F10 steps 2–4, F11 step 3, F13 steps 1–2, F14 all (three frames on the real UI: sidebar tree, tab strip, dock).

## Order of work for ui

F2, F3, F4, F5 (core of the redesign) → F7, F8, F6 → F9, F10, F11, F12 (after the section frames of T-296) → F1, F13, F14.
