# Improve Team Card And Agent's Icon · UX brief

> Companion of [spec.md](spec.md). Small change in existing patterns: no new screen, no new state. UI updates Penpot (02 Components role mark, 04 Screens Feature · Team card) and exports PNGs to `design/` in this folder.

## Role mark
- Same box (14×16, 1 px role-colored border, radius-sm, caption font, role-colored letter). Only the letter's vertical position changes: the capital's optical center on the box's center.
- Show the fix on all role letters in the component (L, B, F, U, X, S and a custom role letter).

## Team card row
Order: **mark · name · activity · status (label, then icon)**.

```
┌ Team ──────────────────────────────────────── 4 agents ┐
│ [L]  lead                    review T-2      Working ⟳ │
│ [B]  back-1                  Bash · tests    At gate ⏸ │
│ [F]  front-12                                   Idle ○ │
│ [U]  a-very-long-agent-na…   task T-1           Done ✓ │
└────────────────────────────────────────────────────────┘
```
- Mark → name gap: `--space-sm` (8 px), as in the timeline.
- Columns: name column as wide as the longest name (cap ≈ 24 ch, ellipsis + tooltip), activity column starts at one x, status column right-aligned on the card edge.
- Activity is secondary text (muted), one line, ellipsis; it shrinks first. Empty activity → empty cell.
- Status: label then icon, same colors/icons as the StatusBadge vocabulary (§9.5). UI decides whether it keeps the badge's raised background; keep it consistent with other status displays.
- Row: whole row clickable (opens the agent), hover highlight like other clickable rows; no new keys.

## States to draw
4 agents with mixed name lengths and statuses (Working, At gate, Idle, Done) · one row without activity · card narrowed (activity ellipsized, status intact).
