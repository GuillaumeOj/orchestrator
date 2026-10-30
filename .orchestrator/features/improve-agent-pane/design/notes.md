# Improve Agent Pane · design notes (T-62)

Penpot file **Orchestrator**. Companion of [../ux-brief.md](../ux-brief.md) and [../spec.md](../spec.md). Existing tokens only (`ui/src/styles/tokens.css`); no new token.

## Frames

| Export | Penpot page · frame | Size |
|---|---|---|
| `dock-1-agent-active.png` | 03 Shell & layouts · `Dock · 1512 · agent active` | 1512 × 982 |
| `dock-2-shell-active.png` | 03 Shell & layouts · `Dock · 1512 · shell active` | 1512 × 982 |
| `dock-3-no-workspace.png` | 03 Shell & layouts · `Dock · 1512 · no workspace pane` | 1512 × 982 |
| `dock-4-zoomed.png` | 03 Shell & layouts · `Dock · 1512 · zoomed` | 1512 × 982 |
| `dock-5-overflow-1280.png` | 03 Shell & layouts · `Dock · 1280 · tab overflow` | 1280 × 800 |
| `dock-tab-variants.png` | 02 Components · `Dock tab · variants` | component sheet |

The five dock frames sit in a new row under `Shell · 1512 · shell dock` (y = 3246, 100 px apart, as the other rows of the page). `Shell · 1512 · shell dock` is kept as the "before" reference; the new frames supersede it.

Content of states 1, 2 (ux-brief "States to draw"): tabs in open order `[L] lead` (Working) · `[B] back-1` (Waiting for you) · `zsh · shop-api` · `[F] front-2` (Idle) · `pnpm test` (shell). Workspace above: the project pane `shop-api`. State 1 has `back-1` active (its agent pane header, the "waiting for you" strip and the PTY), state 2 has `zsh · shop-api` active.

## Dock (container, unchanged unless said)

- Under the workspace, full width of the work column (sidebar ↔ inspector). `color.surface.chrome` background, padding `0 space.xs space.xs`.
- Handle: `space.xs` (4 px) tall on top, `color.border.strong` on hover / focus.
- Height: 0.45 of the column by default (spec §3.1). In the 1512 frames: work column 919 px tall (title bar 36, status bar 26 excluded) → dock 414 px, workspace above 505 px.
- Body: `radius.sm` on every corner but the top-left (the active tab sits on it), clip.

## Tab strip

- Height 24 px (as today). Gap between tabs `space.xxs` (2 px).
- Left: the tabs, in open order. Right, pinned: `+` (new shell), then the hint `⌘J hide` (`text.caption`, `color.text.muted`, `space.sm` right padding). In the zoomed state the hint group reads `ZOOMED` · `⌘; z restore` · `⌘J hide` (see State 4).
- **Overflow**: the tabs scroll horizontally inside their own lane (scrollbar hidden); `+` and the hint group don't scroll. When the lane overflows, a 1 px `color.border.subtle` divider separates the lane from `+`, so the clipped tab reads as cut, not as the last one. The active tab is always scrolled into view. No fade, no "n more" counter (decision: not in the brief, ⌘; ] / [ and the palette already reach every tab).

## Dock tab (02 Components · `Dock tab · variants`)

Shared by both kinds: height 24, top corners `radius.sm`, font `text.label` (12 px / 500), padding `0 space.sm` on the label, `0 space.xs` on the ×.

| Part | Agent tab | Shell tab |
|---|---|---|
| lead | role mark: AgentChip mark, 14 × 16, 1 px border in `color.role.n`, letter `text.caption` in `color.role.n` | — |
| gap | `space.xs` (4 px) between mark, name and status icon | — |
| label | short name (agent id minus `<feature-slug>-`), `color.text.secondary` | title as today (`zsh · <project>`, `trust · <project>`, the command), `color.text.secondary` |
| status | StatusBadge icon only (compact), `size.icon` (16 px) box, glyph in `color.status.<state>` | — |
| × | `color.text.muted`; tooltip "Close tab · the agent keeps running" | `color.text.muted`; tooltip "Close shell" |
| tooltip on the tab | full agent id · status label (e.g. `improve-agent-pane-back-1 · Waiting for you`) | the title |

States:

| State | Background | Label | Other |
|---|---|---|---|
| default | `color.surface.chrome` (blends with the dock) | `color.text.secondary` | × `color.text.muted` |
| hover | `color.surface.pane` | `color.text.primary` | × `color.text.secondary` |
| active | `color.surface.raised` (joins the body) | `color.text.secondary` | — |
| active + focused (keyboard in the dock) | `color.surface.raised` | `color.text.primary` | — (as today) |

- Status icons drawn: Working `color.status.working`, Waiting for you `color.status.waiting-for-you`, Idle `color.status.idle`, Suspended `color.status.suspended`, Error `color.status.error`. At gate (`color.status.at-gate`) follows the same rule. No blinking, no count (brief).
- **Long name**: the tab caps at 200 px; the name ellipsizes, mark, status icon and × never shrink. Shell titles ellipsize the same way. Decision: 200 px keeps ~6 tabs visible at 1280 with the hint group, and fits every `<role>-<n>` short name without cutting.

## Body of an agent tab

The agent pane as in 04 Screens · `Agent pane · 1512`, minus the workspace pane header (number, kind, title, zoom, close): first row = agent header (role chip · status badge · model · effort · ctx · session · actions), then its strips (here the "waiting for you" strip of back-1), then the PTY. Background `color.surface.pane`, same paddings as the agent pane.

## States

1. **Agent tab active** — workspace = project pane `shop-api` (single pane, focused border off since the dock has focus); dock 0.45; `back-1` active and focused.
2. **Shell tab active** — same layout; `zsh · shop-api` active, the terminal body; the agent tabs keep their status icons (back-1 still waiting).
3. **No workspace pane** — the dock fills the work column: no handle (nothing to resize against), no empty-workspace message; the tab strip starts at the top of the column.
4. **Zoomed** — the dock covers the whole work column (the workspace is not drawn); no handle; the right group reads `ZOOMED` (`text.caption`, `color.accent.default`, as the pane header's tag) · `⌘; z restore` · `⌘J hide`. Decision: no "n of m" after ZOOMED, the dock is a single zoom target.
5. **Overflow at 1280** — 12 tabs (7 agents, 5 shells) with the rail + overlay inspector layout of `Shell · 1280`; the active tab (`ui-1`) is the 9th, scrolled into view, the first ones cut on the left; divider before `+`.

## Decisions beyond the ux-brief

- Tab max width 200 px with ellipsis (brief says "long name ellipsized", no size).
- Hover background `color.surface.pane` (existing token, between chrome and raised).
- Overflow divider (1 px `color.border.subtle`), no fade or counter.
- No drag handle when the dock fills the column (states 3 and 4).
- `ZOOMED` without "n of m".
