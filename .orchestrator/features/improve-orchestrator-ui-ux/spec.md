# Improve Orchestrator UI UX · Feature spec

> Feature `improve-orchestrator-ui-ux` · Integration branch `orch/improve-orchestrator-ui-ux/integration` · Team **Design sprint** (lead, ux, ui): this feature delivers the **design**: audit, interaction rules, FigCraft frames and the implementation hand-off. The code is a follow-up feature run by a team with a frontend. Companions: [ux-brief.md](ux-brief.md), [api-contract.md](api-contract.md).

## 1. Problem (from the user's brief)

1. **Buttons and shortcut hints look alike.** `Kbd` hints are rendered in ~40 components (title bar, banner, status bar, dock, pane headers, settings, chat, palette…). The user can't tell a clickable control from a hint that only explains a key, and the hints eat space.
2. **Duplicated actions.** The same action is offered in several places on one screen. Example: a suspended agent shows Resume/… in the app banner **and** in the agent pane's header/strip. The whole app must be checked, not only agent panes. (Audit, [audit.md](audit.md) §0: the app banner shows only app-wide conditions; the suspended-agent duplicate is the agent pane's strip + header, plus the Inspector.)
3. **Settings live in workspace tabs.** App Settings (⌘,: 10 sections), Project settings (7 sections, Intake included) and the Week planner open as panes (`openSettings`, `openProjectSettings`, `openSingleton('planner')` → `addPane(…, 'tab')`), mixed with work tabs.
4. **Shell titles are frozen.** A shell dock tab keeps the path it opened in (`zsh · <project>`); it doesn't follow `cd`.
5. **The dock mixes agents and shells** in one tab strip; with many agents from several projects it is hard to find one.
6. **The main area has no project context.** Panes of every project sit in one tab strip.
7. **Tabs shrink** (ellipsized titles) when many are open, and there is no way to reach hidden ones but cycling.
8. **Dropdowns are native `<select>`** (27 of them): they don't match the app's text inputs and look different per OS.
9. **The sidebar tree forgets its state**: expanded/collapsed projects and features (`Sidebar.svelte` `collapsed`, in-memory) reset at each launch.
10. **Splits are not used**: the app window is often not full screen; the user works with one pane at a time.

## 2. Goals

- Every on-screen element is clearly **either a control or information**. Shortcut knowledge moves out of the chrome.
- **One home per action** on any given screen.
- Settings-like screens are **floating windows** above the app; the workspace only holds work.
- The center shows **one pane**, scoped to **one project**, with tabs that never shrink.
- The dock is split into **Agents** and **Shells** views; agents filterable by project; shells titled by their live directory.
- A shared **Select** component (and its menu) replaces every native dropdown.
- UI state the user shapes (tree expansion, project context, dock view and filter) **survives a restart**.

## 3. Scope (design decisions; details in ux-brief.md)

### 3.1 Controls vs hints (problem 1)
- Rule: a shortcut is never shown inline in the chrome as a pill next to/inside a button. It goes in the control's **tooltip** (`Resume · ⏎`), in the **palette** (right-aligned, as today), and in a **Keyboard shortcuts** sheet (⌘/ and Help menu), which absorbs the Settings › Keyboard section.
- Allowed inline exceptions, styled as plain muted text and never as a button: the empty-state of a pane (one line, e.g. "⌘K to open anything"), and the leader-mode overlay while the leader key is held.
- Buttons have one look per level (primary / secondary / ghost / icon) from the existing tokens; a `Kbd` pill is not interactive and has no hover state.

### 3.2 One home per action (problem 2)
- **UX audits every screen** (title bar, banner, sidebar/icon rail, status bar, dock, every pane, chat column, inspector, dialogs) and lists each action with every place it appears. Output: `audit.md`, a table *action → places today → single home → removed from*.
- Placement rules the audit applies:
  - The **banner** is for app-wide conditions only (no agent core, Claude Code setup problem, weekly / 5-hour limit reached, Governor in Red, off-hours work without AC power, core warnings: audit §3). A condition that belongs to one agent/feature/project is shown **in its pane** (strip) and as a badge in the sidebar, not in the banner.
  - A pane's **header** holds identity and pane-wide actions; a **strip** holds the action that resolves the current state (Resume on a suspended agent). Never both for the same action.
  - The **inspector** shows details and secondary actions, not a copy of the pane's primary action.
  - Toolbar vs context menu: a frequent action in the toolbar, the rest in a `⋯` menu (uses the new Menu component), not both.

### 3.3 Settings and Week planner as floating windows (problems 3)
- **App settings** (⌘,), **Project settings** (from the project pane's Settings button, the palette, and MCP deep links with a project) and the **Week planner** open as an **in-app floating window**: centered over the app, dimmed backdrop, title bar with the window name (and the project for Project settings), close ×, Esc closes, section nav on the left as today. Not a workspace tab, not an OS window.
- One window at a time; opening another replaces it. Deep links (`openSettings(section, server)`, banners, decision cards) open the window on that section.
- Sizes: settings 960 × min(760, viewport − 80); planner 1100 × same; both shrink with the app window (min 720 wide), content scrolls inside.
- Unsaved edits: autosaved sections need no confirm on close; sections with a Save/Revert footer (settings-brief §0.3) ask "Discard changes?" when closed or left while dirty (design-notes §3.3).

### 3.4 One pane, one project (problems 6, 7, 10)
- **No splits.** The workspace is one tab strip and one pane. Split right/below (leader v / s), the pane numbers (⌘1…9 become "tab 1…9") and pane zoom go away; nvim and shells opened "in a split" go to a tab (nvim) or the dock (shells). A saved layout with splits restores as tabs in reading order.
- **Project context dropdown**, always shown at the left of the workspace tab strip: each project, plus **All projects**. It filters the tabs: project-bound panes (project, feature, board, diff, team canvas, editor of that project) show only in their project; a pane with no project (an editor opened outside a project, a team canvas opened from All projects, the weekly report) is app-wide; app-wide panes (Mission control, Setup) always show; Mission control filters its rows by the context (its own ProjectFilter goes, audit §2.4). The weekly report is app-wide too. Opening a pane of another project switches the context to it, except in All projects, which stays All (design-notes §5.1 rule 3). Clicking a project in the sidebar sets the context.
- **Tabs keep their full title** (min width = content, max ~280 px then ellipsis only for very long names). When they overflow, **‹ ›** scroll buttons appear at the right end of the strip (disabled at each end), plus wheel/trackpad scroll; the active tab is always scrolled into view.

### 3.5 Dock: Agents / Shells (problems 4, 5)
- A **view dropdown** at the left of the dock strip: **Agents (n)** / **Shells (n)**. When Agents: a second dropdown **project**: All projects / each project with agents. Tabs show only the selected view and filter. A waiting dot sits on the closed Select whose current choice hides the waiting agent: the project Select when the agent belongs to another project, the view Select only in the Shells view (one agent never lights two Selects; design-notes §12.7).
- Opening an agent switches the dock to Agents (filter to its project, or keeps All); opening a shell switches to Shells. ⌘; ] / [ cycle within the current view.
- **Shell tab title follows the shell's working directory**: `zsh · ~/dev/orch/ui` (home shortened to `~`, path truncated from the left beyond ~32 chars, full path in the tooltip). Source: OSC 7 (cwd) with a fallback to OSC 0/2 (title) — see api-contract.md. Command shells (`runInPane`, trust) keep their fixed title.

### 3.6 Select component (problem 8)
- One **Select** (single value, optional search for > 8 options, optional group headers, disabled options with a reason) and one **Menu** (actions) built on the same popover; same height, border, radius, focus ring and error state as the text input. Keyboard: ↑/↓, Home/End, type-ahead, Enter/Space, Esc; opens below or flips above; aria `combobox`/`listbox`. Used by the dock and workspace dropdowns and replaces every native `<select>` (27 at the audit base, 34 after Intake).

### 3.7 Remembered state (problem 9)
- Persisted across restarts: sidebar tree expansion (per project and feature), workspace project context, dock view and agent project filter. Contract in api-contract.md.

### Out of scope
- Code: implementation is the follow-up feature, from `design-notes.md` and the exported frames.
- New features, theme/tokens rework, the terminal look, Mission control's own layout beyond the dedup rules, the chat column's content.

## 4. Deliverables

1. `audit.md` (ux): every action and hint per screen, duplicates, and the decision for each (§3.1, §3.2).
2. FigCraft (the project file `Orchestrator.figcraft` on Google Drive, ADR-012; **never** the repo copy `docs/design/Orchestrator.figcraft`):
   - 02 Components: Select (closed, open, search, groups, disabled, error, focused), Menu, Tab strip with overflow buttons, Floating window, Button vs Kbd/hint styles, Dock view/filter dropdowns, shell tab with live title.
   - 03 Shell & layouts: the app at 1280 and 1512 with the project context dropdown and one pane; tab overflow; dock in Agents (All / one project) and Shells views; waiting dot per design-notes §12.7 (project Select in Agents, view Select in Shells).
   - 04 Screens: Settings window, Project settings window, Week planner window, Keyboard shortcuts sheet; every pane/strip the audit changes (at least: agent pane suspended/waiting with banner gone, project pane, feature pane, Mission control, status bar, title bar).
3. PNG exports of every new/changed frame under `.orchestrator/features/improve-orchestrator-ui-ux/design/`, committed.
4. `design-notes.md` (ux, after the frames): the implementation hand-off for the follow-up feature: per frame, which components/panes change, state and persistence keys, keyboard changes (removed: split, pane numbers, zoom; added: ⌘/), migration of saved layouts, test list.

## 5. Acceptance

1. `audit.md` covers every component in `ui/src/components` and `ui/src/panes` that renders an action or a `Kbd`, and each duplicate has one decided home.
2. Every brief item (§1, 1–10) maps to at least one exported frame or a design-notes rule.
3. No exported frame shows a shortcut pill inside or next to a button outside the allowed exceptions (§3.1); no action appears twice on one frame.
4. Frames use the existing tokens (`docs/design/tokens.json`) and components; new components are in 02 Components and used by the screens.
5. PNGs are committed under `design/`; the Drive `.figcraft` holds the frames; the repo `.figcraft` is untouched.
6. `design-notes.md` is precise enough for a frontend to implement without reopening the brief (file/function anchors in `ui/src`).

## 6. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-288 App audit: actions, hints, duplicates | lead (ux had no slot) | — | `audit.md`; reads `ui/src` and the current frames in `docs/design/img` |
| T-289 Components: Select, Menu, tab overflow, floating window, button vs hint, dock dropdowns | ui | T-288 | 02 Components in the Drive figcraft; PNGs |
| T-290 Shell, dock and screens | ui | T-289 | 03 and 04 frames per §4.2; PNGs; hand-off per frame |
| T-291 Design notes for the implementation | ux | T-290 | `design-notes.md`; reviews T-290 frames against `audit.md` |
| T-293 Polish the 03/04 frames | ui | — | layout glitches, sample data, ux's review nits |

## 7. User review, round 1 (2026-11-09, PR declined) — decisions

The branch was rebased on main (bd3c92d), which brings the resizable Overwatch column (`allow-overwatch-menu-resizing`, #47) and Intake (#48: Project settings › Intake).

| # | User's point | Decision |
|---|---|---|
| R1 | The Overwatch (chat) column is resizable like the dock: show its handle | Every frame with the chat column shows the column's resize handle as designed in `.orchestrator/features/allow-overwatch-menu-resizing/design/` (same visual rule as the dock's handle) |
| R2 | Project pane worktree row: "Remove…" → red bin icon | Icon button, `color.danger`, tooltip "Remove worktree…" |
| R3 | Project pane worktree row: "Diff" → +/- icon | Icon button (diff glyph ±), tooltip "Diff" |
| R4 | Project pane header: Fetch, Shell, Diff main under a "more" menu; Settings = gear icon; Remove project… = bin icon | Header: gear icon (tooltip "Project settings"), bin icon in danger color (tooltip "Remove project…"), `⋯` Menu with Fetch, Shell, Diff main (+ Trust in a pane when untrusted). Same pattern for every pane header: identity left, icon actions right, `⋯` last |
| R5 | Title bar Decisions: same gap between count and label as between label and icon | Fix the spacing (one gap token for icon · label · count) |
| R6 | Tab overflow end is buggy (shadow on the clipped tab, a sliver of the next tab, then the buttons) | The strip clips at the scroll buttons' left edge: one fade on the clipped tab, nothing drawn between it and ‹ ›, the buttons on an opaque background |
| R7 | Suspended agent frame: the open menu isn't clearly tied to the ⋯ | Show the ⋯ trigger in its pressed/open state, the menu anchored under it (right-aligned), and a pointer on the trigger |
| R8 | Week planner screens not visible on 04.6 | They were placed at y = 20000. Every frame of this feature goes in a labelled "Improve UI UX" row at the **top** of its page (old frames move below and are tagged "superseded"), on 03, 04.x, 05, 06 |
| R9 | Feature pane: Publish to Notion, Board, Canvas under a ⋯; Pause, Stop, Delete as icons | Header: icon buttons Pause/Resume, Stop, Delete (danger) with tooltips, then `⋯` with Board, Canvas, Publish/Refresh in Notion, Open in Notion |
| R10 | Feature pane: show each doc's path (when in the repo) under the doc tabs | A muted path line under the doc tabs: repo-relative path, Open in nvim as an icon at its end |
| R11 | Select next to a text input must have the same height | Select (regular) = text input height; the compact variant only in dense strips (tab strip, dock). Check every frame where they sit side by side |
| R12 | Keyboard shortcuts screens not visible | Same cause as R8 (page 05): move them up |
| R13 | Feature pane with an open PR (link + CI state) is missing | New frame: feature pane with an open PR: PR line (link, CI dot and count, checks popover open), in running and in review-done states |
| R14 | Design the other settings too (04.7), full redesign | Every section of both windows gets a frame: Settings › Claude Code, Integrations, Overwatch, Permissions, MCP servers, Terminal & editor, Pacing policy, Memory, Builds; Project settings › General, Files, Task backend, Intake, Allocation, Notion, MCP overrides. Placeholders today (Claude Code, Terminal & editor, Memory, General) get a real design from what the app already stores; ux writes the section brief first |
| R15 | Rework the Flows page | 06 Flows is redrawn for the new UX (single pane, project context, dock views, floating windows, ⋯ menus); ux writes the flow list first |

### Tasks, round 2

| Task | Owner | Depends on |
|---|---|---|
| Review fixes on the existing frames (R1–R13) | ui | — |
| Settings section brief + flows list | ux | — |
| Settings sections and 06 Flows frames (R14, R15) | ui | both above |
| Design notes and audit updated for round 2 | ux | the frames task |
