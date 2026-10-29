# Improve the end of a feature · design frames

Drawn in Penpot (file Orchestrator, page `04 Screens`, row below the Feature screens), Dark theme, from [ux-brief.md](../ux-brief.md). No new token.

| Image | Penpot frame | Shows |
|---|---|---|
| [ui-pr-line-states.png](ui-pr-line-states.png) | `Feature · PR line · states` (x 1612, y 3740) | Card "PR and artifacts" at 440px, one line per state: 1b legacy, 2 checking, 3 grey 0/6, 4 yellow 2/6, 5 green 6/6, 7 no checks, 8 behind, 9 conflicts, 10 unknown, 11 ↑ to push, 12 rebased (+ behind + to push), 13 stale, 14 refreshing, 15 merged, 16 closed, and the worst case truncated (§1.6: steps 1–2, step 3, steps 3–5). |
| [ui-ci-tooltip.png](ui-ci-tooltip.png) | `Feature · CI tooltip · states` (x 3224, y 3740) | State 6, CI red 4/6 with the popover open (6 jobs, first row hovered), and the overflow popover (20 jobs, scrolled 5 rows, 12 rows max, header and footer fixed). |
| [ui-pr-open.png](ui-pr-open.png) | `Feature · PR open · 1512` (x 1612, y 4700) | The Feature screen with the run state badge `PR open` (cyan outline, as `Executing`), the PR line (red 4/6) in the card and the timeline line `PR gate → PR open`. |
| [ui-update-gate.png](ui-update-gate.png) | `Inbox · PR update gate · states` (x 4836, y 1100) | Gate `push_pr_update`: list card + detail, normal (`Approve: push 2 commits`) and force-push (`force-push` tag in the list and header, error warning block, `Approve: force-push`). |

## Tokens used

- **Surfaces**: `color.surface.pane` (cards, detail), `color.surface.raised` (popover), `color.surface.selected` (selected list card, hovered job row), `color.surface.chrome` (frame backdrop).
- **Borders**: `color.border.subtle` (cards, dividers), `color.border.strong` (popover, scrollbar thumb), `color.accent.default` (selected list card), `color.status.error` (warning block, tag, Reject).
- **CI dot** (filled circle, `size.status-dot`, no icon): ok `color.status.done`, error `color.status.error`, running `color.status.queued`, waiting / none `color.status.idle`.
- **PR line text**: link `color.accent.default` (`text.label`); counts, `checked`, `up to date`, `closed`, stale text `color.text.muted` (`text.caption`); `behind` / `rebased` `color.status.waiting-for-you`; `conflicts` and the stale glyph `color.status.error`; `merged` `color.status.done`.
- **PR open badge**: `color.status.working` (outline and label, `text.label`), like `Executing`.
- **Type**: `text.title`, `text.heading`, `text.body-strong`, `text.label`, `text.caption`, `text.mono-sm`, `text.icon` (refresh 󰑐 U+F0450, error 󰀦 U+F0026, shield-check 󰕥 U+F0565).
- **Spacing / shape**: `space.xs`, `space.sm`, `space.md`, `space.lg`, `radius.sm`, `radius.md`, `shadow.overlay` (popover).
- **Components reused**: `Button` (Approve / Reject, Reject with an error stroke), `Key hint`.

## Notes for front

- The popover is 400px wide (320–400), rows 24px, header `text.caption` so the longest count line fits; the cap footer variant (`+N more on GitHub ↗`) uses the same footer.
- Truncation frames show the order of §1.6; `checked` and the refresh button sit at the right edge via a spacer.
- Only the Dark theme is drawn; the Light theme comes from the same tokens.
