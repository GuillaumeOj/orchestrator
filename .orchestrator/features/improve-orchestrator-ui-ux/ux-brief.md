# Improve Orchestrator UI UX · UX brief

> Companion of [spec.md](spec.md). Design source: the project file `Orchestrator.figcraft` on Google Drive (ADR-012). Export every new/changed frame as PNG to `design/` in this folder.

## Layout (1280, after)

```
┌ title bar ─────────────────────────────────────────────────────────────────┐
│ banner (app-wide conditions only)                                           │
├ sidebar ─┬ [orch ▾] │ Overview │ orch · project │ feat-x │ board │ … ‹ › ─┬ chat ┤
│ tree     │                                                              │      │
│ (state   │                 one pane (no splits)                         │      │
│ kept)    │                                                              │      │
│          ├══════════════════════ drag handle ═══════════════════════════┤      │
│          │ [Agents 4 ▾] [orch ▾] │ L lead ⟳ │ B back-1 ⏸ │ U ui-1 ○ │ ‹ › + │      │
│          │ agent pane (header, strip, PTY)                              │      │
├──────────┴──────────────────────────────────────────────────────────────┴──────┤
│ status bar (information only)                                               │
└─────────────────────────────────────────────────────────────────────────────┘
          ┌ Settings ─────────────────────────── × ┐  ← floating window, dimmed backdrop
          │ Claude Code   │ section content         │
          │ Integrations  │                         │
          │ …             │                         │
          └─────────────────────────────────────────┘
```

## Controls vs information

- A control looks like a control: button (primary/secondary/ghost), icon button (with tooltip), Select, tab, link. Hover and focus states on all.
- Information looks static: text, badges, status icons, `Kbd` pills. `Kbd` has no hover, no pointer cursor, and appears only in tooltips, the palette, menus (right-aligned), the Keyboard shortcuts sheet, an empty pane's one-line hint and the leader overlay.
- Remove: inline hints in the title bar, status bar, dock (`⌘J hide`), pane headers (`⌘; z restore`…), strips and settings rows. Their key goes to the control's tooltip: `Hide dock · ⌘J`.
- Keyboard shortcuts sheet (⌘/ and Help › Keyboard shortcuts): a floating window, grouped (Navigation, Workspace, Dock, Agents, Chat), searchable; replaces Settings › Keyboard.

## Duplicates (audit rules)

For each action, list where it appears today, pick one home with these rules, remove the others:
1. Banner = app-wide only. Per-agent/feature/project states → that pane's strip + sidebar badge.
2. Pane header = identity + pane-wide actions; strip = the action that resolves the current state. Never the same action in both.
3. Inspector = details + secondary actions, never the pane's primary action.
4. Toolbar = frequent actions; `⋯` Menu = the rest. Never both.
5. Status bar = information; a click may open the related window (pacing → Week planner), no buttons.
6. The palette lists every command (it is the catalogue, not a duplicate).

Known case to settle first: suspended agent (banner + header + strip). Then the whole app.

## Floating windows

- Settings, Project settings (title "Project settings · <project>"), Week planner, Keyboard shortcuts.
- Centered, radius and shadow of the overlay tokens, backdrop dims the app (click on it closes), Esc closes, focus trapped inside, focus goes back to where it was.
- Left section nav (Settings, Project settings), content scrolls; the Week planner keeps its current content, laid out for the window width.
- Opening one closes the open one. A deep link opens on its section (and selects the server, as today).

## Workspace

- **Project context Select**, left of the tab strip, always shown: `All projects` + projects (status dot when a project has something waiting). Selecting filters tabs (spec §3.4). Last choice is kept.
- Tabs: full title, close × on hover/active, icon by kind. Overflow: tabs scroll; **‹ ›** at the right end, disabled at each end; the active tab scrolled into view; a tab opened off-screen scrolls in.
- Empty (no tab for this context): a one-line muted hint and two buttons (Open project overview, New feature) — no key pills besides the one-line hint.

## Dock

- **View Select**: `Agents (n)` / `Shells (n)`; waiting dot on it when a hidden agent needs the user.
- **Project Select** (Agents view only): `All projects` + projects with agents.
- Tabs as today (agent: role mark · short name · status icon; shell: title). Shell title = `zsh · <cwd>` live, `~` for home, left-truncated, full path in the tooltip.
- Overflow like the workspace (‹ ›), then `+` (new shell; in Agents view it is hidden).
- Empty view: "No shell open" + New shell button; "No agent running in <project>".

## Select / Menu component

- Closed: same box as the text input (height, padding, border, radius), value + chevron; states default, hover, focused, open, disabled, error; compact variant (dock and tab strip).
- Open: popover list, max 320 px high then scroll, selected ✓, hover/keyboard highlight, group headers, disabled option with tooltip reason, optional search field at the top (> 8 options), status dot or count per option when needed.
- Menu: same popover, items with icon, label, right-aligned `Kbd`, separators, a destructive item style.

## States to draw

1. App 1280 and 1512: context `orch`, 4 tabs, one pane (project pane), dock Agents view (All projects) with 3 agents.
2. Tab overflow (12 tabs) at 1280, both scroll buttons states.
3. Context `All projects`; context switch open (Select open).
4. Dock Shells view with 2 shells, live titles; dock Agents filtered to one project; waiting dot on the view switch.
5. Agent pane suspended and waiting-for-you after dedup (banner without the agent condition).
6. Settings window (Integrations), Project settings window (Files), Week planner window, Keyboard shortcuts sheet; each at 1280 and a narrow app window (1024).
7. Every other pane the audit changes.
8. Components page: Select (all states), Menu, tab strip overflow, floating window, button levels vs Kbd.

## Keys

- Removed: leader v / s (split), ⌘1…9 as pane numbers (now: tab 1…9 of the current context), ⌘; z zoom.
- Added: ⌘/ Keyboard shortcuts.
- Kept: ⌘K palette, ⌘, Settings (window), ⌘J dock, ⌘; ] / [ dock tabs (current view), ⌘W close tab, ⌘⇧T reopen.
