# Improve Agent Pane · Feature spec

> Feature `improve-agent-pane` · Integration branch `orch/improve-agent-pane/integration` · UI change (Penpot pass, then code), a terminal drawing fix that may touch the core. Companions: [ux-brief.md](ux-brief.md), [api-contract.md](api-contract.md).

## 1. Problem

1. An agent opens as a new pane **to the right** of the open panes (`openAgent` → `addPane(…, 'right')`, a tab only from 3 visible panes). Agents crowd the workspace and squeeze the project, feature and board panes.
2. Each agent is **a pane of its own**, next to the others, not a tab.
3. Sometimes an agent pane is **not fully drawn** (parts of Claude Code's UI missing) until the user resizes the window.
4. The project pane's **Shell** button opens the shell as a split to the right (`openShell({ …, how: 'right' })`), not in the shell dock.

**Goal:** one panel at the bottom of the work column, the one shells already use (the dock, §9.2 / FR-1003), holds **every shell and every agent**, one tab each. The workspace above keeps the project, feature, board, Overview, Inbox… panes. An agent is drawn at its real size the moment its tab shows.

## 2. Today (HEAD bd5797c)

- `App.svelte`: `.work` column = `Workspace` (tiling tree, `lib/layout.js`) over `Dock` (only when `app.dock.open && app.dock.tabs.length`).
- `app.dock = { open, height, tabs: [paneId], active }`, persisted with the layout (`saveLayout` / `restoreLayout`). `addDockPane`, `toggleDock` (⌘J), `cycleDock` (⌘; ] / [), `focusPane`, `closePane` already know dock panes (`inDock`).
- `Dock.svelte` renders only `TermPaneView` for the active tab, inside `{#key app.dock.active}`: the active tab remounts on every switch (the xterm is kept in `terms`, its host re-parented).
- `PaneFrame` renders `AgentPane` for an agent in the workspace, with the pane header (number, kind, title, zoom, close).
- `toggleZoom` on a focused dock shell sets `app.zoomed` to it: `Workspace` then renders that pane too, while the dock still shows it (one xterm, two hosts).
- Terminal sizing: `TermPane` (`lib/term.js`) + `mountTerm`. For an agent, `start()` calls `agent_attach(id, rows, cols)`; the core (`crates/orch-app/src/core.rs` `agent_attach`) resizes the PTY to `cols-1` then `cols` **back to back** to make the idle TUI redraw. A re-shown agent tab (already `alive`) is not attached again and gets no resize when its size did not change. The shell-side fix of `fix-shell-pane-on-opening` (settle frame, cell-change refit, delayed nudge) applies to shells only for the nudge.

## 3. Scope

### 3.1 One bottom panel for shells and agents (the dock)

- **Agents open in the dock.** `openAgent` adds the agent as a dock tab (`addDockPane`) and focuses it; an agent already open (in the dock) gets its tab focused. Every path that opens an agent goes through it: sidebar, icon rail, Overview, Feature team card, project pane, palette `Open <agent>`, leader w (`nextWaiting`), Inbox/notification links.
- **Mission control sessions too** (user decision, 2026-11-02, after main brought P4.2): `openSession` opens the agent as a dock tab in watch mode with `over` set. Esc returns to Mission control, ⏎ takes over, and an agent already open gets its dock tab back in watch mode (T-67).
- **Shells open in the dock**: `openShell()` without `how` (unchanged), the project pane's **Shell** button (drop `how: 'right'`), the Inspector's **Shell here** (unchanged), `runInPane` (gh auth login, claude /mcp) and `openTrust` (trust dialog). These shells keep their title (`trust · <project>`, the command).
- Stay in the workspace: nvim (`openEditor`), every non-terminal pane, and the explicit **Split right / Split below (shell)** commands (leader v / s), which keep making a shell pane where the user asked for one.
- The dock shows the active tab's content: `TermPaneView` for a shell, `AgentPane` (its header with role, status, model, ctx, session, actions and its strips) for an agent. No `PaneFrame` header in the dock: the tab carries the title and the close button.
- Tab content per [ux-brief.md](ux-brief.md): an agent tab shows its role mark, short name and status icon; a shell tab its title as today.
- **No workspace pane open** (`app.layout` empty) and the dock open: the dock fills the work column (no empty-workspace message squeezed above it). Closing the last dock tab brings the empty workspace back.
- **Zoom** (⌘; z) on a dock tab maximizes the dock over the whole work column; ⌘; z again (or focusing a workspace pane) restores it. A dock pane is never rendered by `Workspace` (fixes the double mount), so zoom never crosses the dock boundary: when `focusPane` / `focusNumber` focus the other side (a workspace pane while the dock is zoomed, a dock tab via leader j, ⌘; ] / [ or `openAgent` while a workspace pane is zoomed), they restore instead of moving `app.zoomed` as they do today.
- Focus and keys: ⌘J shows/hides the dock (a new shell when empty, as today); ⌘; ] / [ cycle all dock tabs, agents included; leader j from the bottom workspace row goes into the dock, leader k goes back (as today); ⌘1…9 stay for workspace panes; ⌘W closes the focused dock tab (an agent tab only detaches: the agent keeps running, as closing its pane does today); ⌘⇧T reopens a closed agent tab in the dock.
- **Restore**: a saved layout with agent panes in the workspace restores them as dock tabs, after the saved dock tabs, in their order, the focused one active, and the dock opens (`restoreLayout` would otherwise keep `saved.dock.open` and hide them). Shells and editors in splits stay where they were (a split shell cannot be told apart from one the user asked for). A saved layout with dock tabs but no workspace pane restores its dock as is (today `restoreLayout` returns early on an empty `layout`, drops the dock and adds Overview); Overview is added only when both come back empty.
- Default dock height 0.45 of the column (agents need rows). Every layout saved so far stores the old default 0.3, so: a saved layout without the new `dock.v: 2` marker and a height of exactly 0.3 restores at 0.45 (once; `saveLayout` then writes `v: 2`); any other saved height is kept. The existing clamp (0.12–0.8) and drag/keyboard resize stay.

### 3.2 An agent is drawn at its real size

Every time an agent's tab shows (first open, switching back to its tab, the dock shown again, a resumed agent, the app restored), without any user action, the xterm grid matches its box and Claude Code redraws its whole screen for that size.

The fix owner first confirms the cause (instrumented build: `localStorage['orch.termDebug'] = '1'` logs fits and PTY resizes) and fixes the real one(s):

- **A1 · The attach nudge is lost.** The core resizes `cols-1` then `cols` back to back: the TUI handles SIGWINCH after both, reads the unchanged size and does not redraw. The shell nudge has a gap (`NUDGE_MS`); the agent one has none.
- **A2 · Attach at a size that is not final.** `start()` attaches with the size of the first fit, before the settle frame or the dock reaching its height; the later real size goes as a plain `agent_resize`, which may arrive while the TUI is mid-draw.
- **A3 · Re-show without a redraw.** A tab switch remounts the pane: the TermPane is `alive`, its size is the same, so nothing is sent; what xterm parsed while detached (or a size another pane forced meanwhile) is left as is.
- **A4 · Cell metrics settle late** (as H2 of `fix-shell-pane-on-opening`): covered for shells, to be checked for agents.

The fix lives in `TermPane` / `mountTerm` (front) and, only if the plan shows it is needed, in the core's `agent_attach` (back, per [api-contract.md](api-contract.md)). Keep the 60 ms fit debounce and the A → B → A flip guard (FR-1003): Claude Code reprints its whole screen on every resize, so a redraw is forced **once per show**, never in a loop.

### Out of scope

The Overwatch / Mission control (P4.2) and its "session over the board"; the agent pane's header and strips themselves (unchanged, only their container moves); the terminal look; detaching a dock tab into a workspace split (not asked); a second dock.

## 4. Acceptance

1. Opening an agent from the sidebar, the Overview, the Feature team card, the project pane, the palette and leader w shows it as a tab of the bottom dock, focused; the workspace above is unchanged. Opening it again focuses its tab.
2. Several agents and shells sit side by side as dock tabs; switching tabs shows one at a time; hidden ones keep running and keep their buffer.
3. The project pane's Shell button, Shell here, a `runInPane` command and the trust shell open as dock tabs. Split right / below still make a shell pane.
4. With no workspace pane, the dock fills the column. Zoom on a dock tab fills the column and restores; no pane is drawn twice.
5. An agent tab is fully drawn (Claude Code's input box at the bottom, its status line, no missing rows or columns) when it first opens, after switching to another tab and back, after ⌘J hide/show, after resume and after an app restart, with no window resize. The task report names the confirmed cause(s) (A1–A4 or another) and how it was confirmed.
6. Window resize, dock drag and zoom still refit agents and shells, with no redraw loop.
7. A saved layout from before this change restores its agents into the dock.
8. Unit tests: `ui/test/dock.test.js` (agents in the dock, open/focus/close/reopen, restore migration, zoom of a dock tab, Shell button and `runInPane` target) and `ui/test/term.test.js` (the agent redraw on show: one forced redraw per show, none when hidden, flip guard intact). `make test` and `make lint` pass; `cargo test -p orch-app` when the core changes.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-62 Penpot: the dock with agent and shell tabs | ui | — | 03 Shell & layouts: dock with mixed tabs (states in ux-brief), dock filling the column, zoomed dock; 02 Components: dock tab (shell, agent + statuses, focused, overflow). Export PNGs to `design/` |
| T-63 Agent drawn at its real size on every show | front (opus, `plan_first`) | — | Confirm the cause, fix in `term.js` / `mountTerm`; the plan says if the core's `agent_attach` must change (then a back task follows, per api-contract) |
| T-64 Agents and shells in the bottom dock | front | T-62, T-63 | `app.svelte.js` (open*, focus, zoom, restore migration, default height), `Dock.svelte`, `App.svelte`, `Workspace.svelte`, `ProjectPane.svelte`, tests. Asks `ui` for a visual review |
| T-66 Dock frames in FigCraft | ui | T-64 | ADR-012: 5 dock states + Dock tab set in [`Orchestrator.figcraft`](https://drive.google.com/open?id=1n4O57ntQDFvv1-pw3flJkmcHh4MeTLVs&usp=drive_fs), PNGs in `design/` |
| T-67 Mission control sessions open as dock tabs | front | — | `openSession`, `leaveWatch`, restore; `mission-pane.test.js`, `dock.test.js` |

`ux`: no separate task; the interaction rules are settled in [ux-brief.md](ux-brief.md). `back`: no work unless T-63's plan shows the core must change.
