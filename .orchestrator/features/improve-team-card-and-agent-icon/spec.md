# Improve Team Card And Agent's Icon · Feature spec

> Feature `improve-team-card-and-agent-icon` · Integration branch `orch/improve-team-card-and-agent-icon/integration` · UI only (no core, no API change). Penpot first (ADR-008), then code. Companion: [ux-brief.md](ux-brief.md).

## 1. Problems

1. **Agent mark off-center.** The role mark (the colored square with the role letter: `L`, `B`, `F`, `U`, `X`, `S`…) shows its letter a few pixels too high. The Penpot component has the same defect, so code and design both need the fix.
2. **Team card hard to scan.** In the Feature pane, Team card, a row is: agent chip (mark + name, 4px gap) · status badge (icon + label) · current activity (tool / task / flag) at the end. Columns do not line up because each item follows the previous one, so the status and the activity move with the length of the agent name.

## 2. Cause (HEAD 68ea76d)

- `ui/src/components/AgentChip.svelte` `.mark`: a 14×16 px box with a 1 px border, `font: var(--text-caption)` (11px / line-height 1.45, FiraCode Nerd Font Mono). The 1.45 line box (≈16 px) is taller than the 14 px content box and the font's ascent/descent are not symmetric around the capitals, so the capital sits above the box's center.
- `ui/src/panes/FeaturePane.svelte:258-267`: each `.arow` is a flex row (`gap: var(--space-sm)`) of `AgentChip` · `StatusBadge` · `<span class="muted ell">` activity. No shared column widths.

## 3. Scope

### 3.1 Agent mark: letter centered vertically
- The role letter is optically centered in the mark: the center of the capital (cap height) sits on the vertical center of the box, within 0.5 px, for every role letter, at 1× and 2× (Retina).
- Fixed once in `AgentChip.svelte`, so every place that uses the chip gets it (Sidebar, Icon rail, Inspector, Overview, Inbox, Agent pane, Board, Team canvas, Feature pane timeline and team card).
- The box size (14×16), border, radius, color and font stay the same. Horizontal centering stays as it is.
- Penpot: the agent chip / role mark component in **02 Components** is fixed the same way, and the instances update.

### 3.2 Team card: a table of agents
Each row, left to right:
1. **Mark** (the role square).
2. **Agent name**, with the timeline's gap between mark and name (`--space-sm`, 8 px; today the chip uses 4 px). The name is the agent id **without the `<feature-slug>-` prefix** (`back-1`, not `improve-team-card-and-agent-icon-back-1`), as the Sidebar already shows agents by node; the full id stays in the row's `Open <id>` tooltip.
3. **Current activity** (today's last column: tool, else `task T-n`, plus ` · flag`), moved right after the name.
4. **Status**, pushed to the far right of the card: **label first, then the icon** (`Working ⟳`), same label, icon and color as today's `StatusBadge`.

Alignment, like a table:
- All marks, names, activities and statuses line up in columns across rows, whatever the length of the names. The name column is as wide as the longest name in the card (capped, see below); the activity column starts at the same x on every row; the status column is right-aligned on the card's right edge.
- Width pressure, in this order: the activity shrinks first and ends with an ellipsis; then a very long name ellipsizes (cap of about 24 characters, full name in the tooltip); the mark and the status never shrink or wrap.
- An agent with no activity keeps an empty activity cell: its status still sits in the status column.
- The whole row stays clickable (opens the agent pane), with its `Open <id>` tooltip. No new keys.

Out of scope: other agent lists (Sidebar, Overview, Board, Canvas), the status vocabulary, the activity text itself, the timeline.

## 4. Acceptance
1. In the running app, each role letter in the mark is centered vertically (≤ 0.5 px off), in the Feature pane, Sidebar and Board; the mark's size is unchanged.
2. In a Feature pane with agents named `lead`, `back-1`, `front-12`, `a-very-long-agent-name`, the activity texts start at the same x and the statuses end at the card's right edge on every row; the status reads label then icon.
3. Gap mark → name in the team card equals the timeline's gap (`--space-sm`).
4. Narrowing the right column ellipsizes the activity first; the status never wraps or moves out of the card.
5. Penpot: the role mark component is fixed and the Feature screen's Team card (04 Screens, P2 row) shows the new row; exports are committed under `.orchestrator/features/improve-team-card-and-agent-icon/design/`.
6. Component tests in `ui/test/teams.test.js` cover the new order (mark, name, activity, status label then icon) and the empty-activity row; `bun run test`, `bun run check`, `bun run lint` pass.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-37 Penpot: center the role mark letter, redesign the Team card row | ui | — | 02 Components role mark, 04 Screens Feature Team card; export PNGs to the feature's `design/` folder |
| T-38 Code: mark centering + Team card table | front | T-37 | `AgentChip.svelte`, `StatusBadge.svelte` (label-then-icon option, existing callers unchanged), `FeaturePane.svelte`, tests |

`ux`: no separate task; the interaction rules are settled in [ux-brief.md](ux-brief.md) (the user's brief fixes the order). `back`: no work (no data change: the row uses the same `v.agents` fields). `front` asks `ui` for a visual review before reporting done.
