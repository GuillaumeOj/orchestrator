# Improve Team Card And Agent's Icon · Design notes (T-37)

Penpot file **Orchestrator** (`65d5ee1e-9774-8045-8008-d16813b441c7`). Exports in this folder:

| Export | Penpot board |
|---|---|
| [role-mark-centering.png](role-mark-centering.png) | 02 Components › Agent chip › `Role mark · centering` (`0fbaf013-b3f5-80a9-8008-dd40aa37e643`) |
| [team-card-states.png](team-card-states.png) | 04 Screens › `Feature · Team card · states` at x 1612, y 3080 (`0fbaf013-b3f5-80a9-8008-dd4387f022f1`) |
| [feature-team-card.png](feature-team-card.png) | 04 Screens › `Feature · 1512` (P2 row), Team card redrawn |
| [status-badge-trailing.png](status-badge-trailing.png) | 02 Components › Status badge, new `Style = trailing` variants (last row) |

## 1. Role mark: letter centered

**Cause.** FiraCode Nerd Font Mono: 1950 units/em, ascent 1800, descent 600, cap height 1377. The line box is centered in the mark, but the capital's center sits `(1800 − 600)/2 − 1377/2 = −88.5` units from the line-box center: **0.045 em too high**. That is 0.50 px at 11 px (code) and 0.55 px at 12 px (Penpot). Changing `line-height` doesn't fix it, because the offset comes from the font metrics, not the leading.

**Fix (code):** in `AgentChip.svelte` `.mark`, add **`padding-top: 1px`**. Nothing else changes: still 14×16, `border-box` (global), 1 px border, `--radius-sm`, `--text-caption`, role color, `inline-flex` centered. The 1 px top padding lowers the content box's center by 0.5 px, so the flex-centered letter moves down 0.5 px. Expected result at 11 px: about 0.0 px off. On Retina, 0.5 px is exactly 1 device pixel. Every caller of the chip (Sidebar, rail, Inspector, Overview, Inbox, Agent pane, Board, canvas, timeline, team card) gets the fix.

**Fix (Penpot):** every main `role mark` (26 mains: Agent chip default and compact ×6 roles, Tree item, canvas `agent` nodes) now has flex `padding-top 1`. All 414 instances on 02, 03, 04 and 06 updated. Measured on an 8× render: **before −0.62 px, after −0.12 px** at 1× (target ≤ 0.5). Penpot's mark is 16×16 at 12 px and code's is 14×16 at 11 px. That gap was there before this task and stays as it is; the fix is the same 0.5 px in both.

Please check it in the running app (acceptance §4.1): zoom the Sidebar/Board marks. If WebKit's baseline rounding leaves it 1 device px high at 1×, keep `padding-top: 1px`; do not use a translate.

## 2. Team card: a table of agents

Row, left to right: **mark · name · activity · status (label, then icon)**.

### Layout (code)

The agent list is one grid, and each row is a subgrid, so the columns line up across rows.

Use a new class for the agent rows (`.trow` below), not `.arow`: in `FeaturePane.svelte`, `.arow` is also the PR row of the "PR and artifacts" card and sits in the shared flex rule (`.fh, .cardh, .tt, .arow, …`). Restyling `.arow` would turn the PR row into a subgrid with no grid parent (label and URL stacked in one column) and give it a hover highlight and pointer cursor although it is not clickable.

```css
.agents {                       /* wraps the rows, inside .card */
  display: grid;
  grid-template-columns: auto minmax(0, max-content) minmax(0, 1fr) max-content;
  column-gap: var(--space-sm);  /* 8px: mark → name, as in the timeline */
  row-gap: 0;
}
.trow {
  grid-column: 1 / -1;
  display: grid;
  grid-template-columns: subgrid;
  align-items: center;
  height: var(--size-row);              /* 28px (today: 22px badge + 8px gap = 30px pitch) */
  padding: 0 var(--space-xs);           /* 4px hover inset */
  margin: 0 calc(-1 * var(--space-xs)); /* text stays on the card's 16px content edge */
  border-radius: var(--radius-sm);
  cursor: pointer;
}
.trow:hover { background: var(--color-surface-raised); }
.trow .name { max-width: 24ch; }               /* <span class="name ell">: FeaturePane's .ell does the ellipsis */
.trow .act  { margin-left: var(--space-sm); }  /* 8 + 8 = 16px name → activity */
.trow .st   { margin-left: var(--space-sm); }  /* 16px min activity → status; the max-content track ends on the card edge */
```

`.st` is a plain `<span class="st">` wrapping `<StatusBadge trailing />`: `StatusBadge` takes no `class` prop, so a class written on the component itself is dropped.

| Column | Content | Width rule |
|---|---|---|
| 1 mark | `<AgentChip compact>` (14×16) | fixed, never shrinks |
| 2 name | the agent id **without the feature slug prefix** (`<node>-<n>`, e.g. `back-1`), `--color-text-primary` | as wide as the longest name in the card, capped at **24ch**, ellipsis. Full id in the row tooltip (`Open <id>`) |
| 3 activity | today's `<span class="muted ell">` unchanged (tool, else `task T-n`, plus ` · flag`), plus the `act` class | takes what is left, **shrinks first** |
| 4 status | `<span class="st"><StatusBadge trailing /></span>`: label then icon | `max-content`, right-aligned on the card's content edge, never wraps |

- Name: agent ids are `<feature-slug>-<node>-<n>` (`agent_id_for`, `crates/agent-runtime/src/team.rs`), e.g. `improve-team-card-and-agent-icon-back-1`. Showing the full id under a 24ch cap would cut every row to the same `improve-team-card-and-ag…`. Strip the `<v.feature.slug>-` prefix (the Sidebar already labels agents by node, `AgentChip name={a.team.node}`); the drawn names (`lead`, `back-1`, `front-12`) are these short names.

- Gaps: mark → name **8 px** (`--space-sm`). Name → activity **16 px**, activity → status at least **16 px** (`--space-lg`, from column gap plus cell margin).
- Width pressure: activity (`minmax(0,1fr)`) gives up space first, then name (`minmax(0,max-content)`, min 0 because of `overflow: hidden`). Mark and status never shrink.
- Empty activity: render the empty `.act` cell anyway, so the status stays in column 4.
- The mark and name are no longer one `AgentChip` with its 4 px gap. Use the compact chip plus a `.name` span, so the 8 px gap comes from the grid. Other chip callers keep 4 px.
- Card: unchanged (`--space-lg` padding, 8 px gap between header and list). Rows are contiguous (row-gap 0), so the hover highlights touch.
- Interaction: unchanged. The whole row is clickable, opens the agent pane, and keeps the `Open <id>` tooltip. No new keys.
- Text sizes: keep today's inherited sizes. Penpot draws name and status at 12 px and activity at 11 px, per the file's convention.

### Status: label then icon, no raised background

**Decision: in the Team card, the status loses the badge's raised background.** It is plain text: label (`--color-text-secondary`), then a 4 px gap (`--space-xs`), then the icon (`--size-icon` box, status color).
- Why: the row hover is `--color-surface-raised`, the same color as the badge pill. On hover the pill would merge into the row, and a pill inside a clickable row reads like a second button. Without the pill, the status column also lines up cleanly on the right edge.
- Consistency: label, icon, color and vocabulary match §9.5. Elsewhere the pill stays (`badge`) and the icon-only form stays (`compact`).
- Code: `StatusBadge` gets an opt-in prop (e.g. `trailing`) that renders `label` then `icon` with `padding: 0; background: none; height: auto`. Existing callers are unchanged.
- Penpot: Status badge variant set, new `Style = trailing` for all 10 states.

### States drawn (team-card-states.png)

- **A · Default, 440 px**: `lead` Working, `back-1` At gate, `front-12` Idle (no activity), `a-very-long-agent-name` Done. Activities start at one x; statuses end on the card's edge.
- **B · Hover** on `back-1`: `surface.raised`, `radius-sm`, 4 px inset.
- **C · Narrowed to 380 px**: activities ellipsize; the name column and statuses are intact.
- **D · 280 px**: the activity column is gone, then the long name ellipsizes; mark and status are intact.
- **E · Name cap**: a 31-character name is cut at 24ch even though the card has room.

The Feature · 1512 screen's Team card uses the same row with the screen's scenario data (lead, back-1, front-1, ui-1).
