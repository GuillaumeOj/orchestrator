# Improve Agent Pane · design notes (T-62)

FigCraft file [`docs/design/Orchestrator.figcraft`](../../../../docs/design/Orchestrator.figcraft) (ADR-012). Companion of [../ux-brief.md](../ux-brief.md) and [../spec.md](../spec.md). Existing tokens only (`ui/src/styles/tokens.css`); no new token.

## Frames

Drawn in T-66 after T-64 was built (differences: "Drawn as built" below). T-62 had no frames: Penpot was unavailable, then ADR-012 moved the design to FigCraft.

| Export | FigCraft page · frame | Size |
|---|---|---|
| `dock-1-agent-active.png` | 03 Shell & layouts · `Dock · 1512 · agent active` | 1512 × 982 |
| `dock-2-shell-active.png` | 03 Shell & layouts · `Dock · 1512 · shell active` | 1512 × 982 |
| `dock-3-no-workspace.png` | 03 Shell & layouts · `Dock · 1512 · no workspace pane` | 1512 × 982 |
| `dock-4-zoomed.png` | 03 Shell & layouts · `Dock · 1512 · zoomed` | 1512 × 982 |
| `dock-5-overflow-1280.png` | 03 Shell & layouts · `Dock · 1280 · tab overflow` | 1280 × 800 |
| `dock-tab-variants.png` | 02 Components · `Improve agent pane · Dock tab` (component set `Dock tab`) | component sheet |

The five dock frames sit in a new row at the bottom of the page (y = 5068, under `Layout notes · P4.2`, 100 px apart). The Dock tab component set is the last section of 02 Components (`Improve agent pane · Dock tab`): 8 variants (Kind agent/shell × State default/hover/active/active focused), the ten status icons of the P1 Status badge (it has no Not started yet icon variant), long names. `Shell · 1512 · shell dock` is kept as the "before" reference; the new frames supersede it.

Content of states 1, 2 (ux-brief "States to draw"): tabs in open order `[L] lead` (Working) · `[B] back-1` (Waiting for you) · `zsh · shop-api` · `[F] front-1` (Idle) · `pnpm test` (shell). Workspace above: the project pane `shop-api`. State 1 has `back-1` active (its agent pane header, the "waiting for you" strip and the PTY), state 2 has `zsh · shop-api` active.

## Dock (container, unchanged unless said)

- Under the workspace, full width of the work column (sidebar ↔ inspector). `color.surface.chrome` background, padding `0 space.xs space.xs`.
- Handle: `space.xs` (4 px) tall on top, `color.border.strong` on hover / focus.
- Height: 0.45 of the column by default (spec §3.1). In the app at 1512 × 982: work column 922 px tall (982 − `size.titlebar` 36 − `size.statusbar` 24; their 1 px borders sit inside those heights, `box-sizing: border-box`) → dock 415 px, workspace above 507 px. At 1280 × 800: column 740 px → dock 333 px. The frames are 2 px shorter (see "Drawn as built").
- Body: `radius.sm` on every corner, clip; the top-left is square only while the first tab is active and the lane is not scrolled (that tab sits on it).

## Tab strip

- Height 24 px (as today). Gap between tabs `space.xxs` (2 px).
- Left: the tabs, in open order. Right, pinned: `+` (new shell), then the hint `⌘J hide` (`text.caption`, `color.text.muted`, `space.sm` right padding). In the zoomed state the hint group reads `ZOOMED` · `⌘; z restore` · `⌘J hide` (see State 4).
- **Overflow**: the tabs scroll horizontally inside their own lane (scrollbar hidden); `+` and the hint group don't scroll. Only while the lane overflows (no space taken otherwise), a 1 px `color.border.subtle` divider separates the lane from `+`, so the clipped tab reads as cut, not as the last one. The active tab is always scrolled into view. No fade, no "n more" counter (decision: not in the brief, ⌘; ] / [ and the palette already reach every tab).

## Dock tab (02 Components · component set `Dock tab`)

Shared by both kinds: height 24, top corners `radius.sm`, font `text.label` (12 px / 500), padding `0 space.sm` on the label, `0 space.xs` on the ×.

| Part | Agent tab | Shell tab |
|---|---|---|
| lead | role mark: AgentChip mark, 14 × 16, 1 px border in `color.role.n`, letter `text.caption` in `color.role.n` | — |
| gap | `space.xs` (4 px) between mark, name and status icon | — |
| label | short name: the team node (`team.node`, as the sidebar's AgentChip), else the agent id, `color.text.secondary` | title as today (`zsh · <project>`, `trust · <project>`, the command), `color.text.secondary` |
| status | StatusBadge icon only (compact), `size.icon` (16 px) box, glyph in `color.status.<state>` | — |
| × | `color.text.muted`; tooltip "Close tab · the agent keeps running" | `color.text.muted`; tooltip "Close shell" |
| tooltip on the tab | full agent id · the sidebar row's wait line (`waitLine`: status label · what it waits on · since, §9.5), e.g. `improve-agent-pane-back-1 · Waiting for you · permission WebFetch · 2 min`; the whole tab shows it, the status icon included (no badge tooltip of its own) | the title |

States:

| State | Background | Label | Other |
|---|---|---|---|
| default | `color.surface.chrome` (blends with the dock) | `color.text.secondary` | × `color.text.muted` |
| hover | `color.surface.raised` | `color.text.primary` | × `color.text.secondary` |
| active | `color.surface.pane` (joins the agent header row, same surface as the body) | `color.text.secondary` | — |
| active + focused (keyboard in the dock) | `color.surface.pane` | `color.text.primary` | — (as today) |

- Status icons drawn: Working `color.status.working`, Waiting for you `color.status.waiting-for-you`, Idle `color.status.idle`, Suspended `color.status.suspended`, Error `color.status.error`. At gate, Queued, Paused, Blocked by limit and Done the same way; Not started yet too in the app (the P1 Status badge has no icon variant for it, so the sheet shows ten). No blinking, no count (brief).
- **Long name**: the tab caps at 200 px; the name ellipsizes, mark, status icon and × never shrink. Shell titles ellipsize the same way. Decision: 200 px keeps ~6 tabs visible at 1280 with the hint group, and fits every `<role>-<n>` short name without cutting.

## Body of an agent tab

The agent pane as in 04 Screens · `Agent pane · 1512`, minus the workspace pane header (number, kind, title, zoom, close): first row = agent header (role chip · status badge · model · effort · ctx · session · actions), then its strips (here the "waiting for you" strip of back-1), then the PTY. Background `color.surface.pane`, same paddings as the agent pane.

## States

1. **Agent tab active** — workspace = project pane `shop-api` (single pane, focused border off since the dock has focus); dock 0.45; `back-1` active and focused.
2. **Shell tab active** — same layout; `zsh · shop-api` active, the terminal body; the agent tabs keep their status icons (back-1 still waiting).
3. **No workspace pane** — the dock fills the work column: no handle (nothing to resize against), no empty-workspace message; `space.xs` (4 px) of `color.surface.chrome` above the tab strip, where the handle was (T-64).
4. **Zoomed** — the dock covers the whole work column (the workspace is not drawn); no handle; the right group reads `ZOOMED` (`text.caption`, `color.accent.default`, as the pane header's tag) · `⌘; z restore` · `⌘J hide`. Decision: no "n of m" after ZOOMED, the dock is a single zoom target.
5. **Overflow at 1280** — 12 tabs (7 agents, 5 shells) with the rail + overlay inspector layout of `Shell · 1280`; the active tab (`ui-1`) is the 9th, scrolled into view, the first ones cut on the left; divider before `+`.

## Decisions beyond the ux-brief

All five accepted by `lead` (T-62).

- Tab max width 200 px with ellipsis (brief says "long name ellipsized", no size).
- Active tab `color.surface.pane`, hover `color.surface.raised` (lead, after T-62 review: the active tab must join the agent body, whose header row is `surface.pane`; `surface.raised` on the active tab left a seam). Supersedes the first "hover = surface.pane" decision.
- Overflow divider (1 px `color.border.subtle`), no fade or counter.
- No drag handle when the dock fills the column (states 3 and 4); a `space.xs` top padding keeps the tab strip 4 px below the column top, as with the handle.
- `ZOOMED` without "n of m".

## Drawn as built (T-66)

Where the frames differ from the text above, they follow T-64 (`Dock.svelte`):

- Role mark: the P1 Agent chip at Size compact (16 × 16 in the file; the code's mark is 14 × 16).
- Status icon: the P1 Status badge at Style icon is 12 px wide in the file; the code's compact badge is `size.icon` (16 px) wide, so each agent tab is 4 px wider in the app.
- Column height: the shell frames draw the bar rules as separate 1 px layers, so the column is 2 px shorter than in the app (dock 414, 332 at 1280).
- Overflow divider: 1 × 12 px, vertically centred in the strip (`.sep`), not the strip's full height.
- Hints: a Key hint keycap (`Kbd`) then the word in `color.text.muted`; `⌘; z restore` is a button with `space.sm` on both sides.
- States 1 and 2: the sidebar and Inspector show the same agents and statuses as the tabs.
- State 5: the overlay inspector is closed and the workspace holds the project pane. Two long shell commands (ellipsized at 200 px) push `ui-1` past the lane's edge. Under the live cap of 1, `ui-1` is the one Working agent (`lead` At gate, `back-2` Queued).
