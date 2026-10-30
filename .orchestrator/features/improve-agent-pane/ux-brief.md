# Improve Agent Pane · UX brief

> Companion of [spec.md](spec.md). No new screen: the existing shell dock (Penpot 03 Shell & layouts) becomes the home of agents too. UI updates Penpot (03 Shell & layouts, 02 Components) and exports PNGs to `design/` in this folder.

## Layout

```
┌ title bar ─────────────────────────────────────────────────────────────┐
│ sidebar │ workspace: project · feature · board · overview … (tiling)   │ inspector
│         │                                                               │
│         ├══════════════════════ drag handle ═══════════════════════════┤
│         │ [L] lead ⟳ │ [B] back-1 ⏸ │ [F] front-2 ○ │ zsh · orch │ + ⌘J hide │
│         │ ┌ agent header: role · status · model · ctx · session · actions ┐ │
│         │ │ strip (waiting / queued / suspended …) when there is one      │ │
│         │ │ Claude Code (PTY)                                              │ │
│         │ └────────────────────────────────────────────────────────────────┘ │
└ status bar ────────────────────────────────────────────────────────────┘
```

- One dock, under the workspace, full width of the work column (between sidebar and inspector), as today.
- Default height 0.45 of the column when nothing is saved; drag the top edge or ↑/↓ on the handle (unchanged).

## Tabs

Order: the order they were opened (a new tab goes last). No grouping by kind.

- **Agent tab**: role mark (AgentChip mark, same as the sidebar) · short name (agent id without the `<feature-slug>-` prefix, full id in the tooltip) · status icon (StatusBadge vocabulary §9.5, icon only, label in the tooltip) · ×.
  - An agent that needs you: its status icon in the waiting color; no blinking, no badge count.
- **Shell tab**: title as today (`zsh · <project>`, `trust · <project>`, the command) · ×.
- Active tab: raised surface (as today). Focused (keyboard focus in the dock): primary text color (as today).
- `+` makes a new shell (unchanged). The right end keeps `⌘J hide`.
- **Overflow**: the tab strip scrolls horizontally; the active tab is scrolled into view; `+` and the hint stay pinned at the right.
- A tab's ×: closes a shell (kills it, as today); closes an agent tab only (the agent keeps running; tooltip "Close tab · the agent keeps running").

## Body

- Agent tab → the agent pane content as today (header, strips, PTY), without the workspace pane header (number, kind, zoom, close): the tab replaces it.
- Shell tab → the terminal (unchanged).

## States to draw

1. Dock with 3 agent tabs (Working, At gate / waiting for you, Idle) and 2 shell tabs, an agent active, a workspace with a project pane above.
2. Same, a shell tab active.
3. No workspace pane: the dock fills the work column.
4. Zoomed dock (⌘; z): fills the column, the tab strip shows a `ZOOMED` tag like the pane header does, and the restore hint `⌘; z restore`.
5. Tab strip overflow (10+ tabs) at 1280 px.
6. Components: dock tab variants: shell, agent × (working, waiting for you, idle, suspended, error), active, focused, hover, long name ellipsized.

## Keys (no new binding)

⌘J show/hide · ⌘; ] / [ next/previous tab · leader j / k in and out of the dock · ⌘; z zoom the dock · ⌘W close the tab · ⌘⇧T reopen.
