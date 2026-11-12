# Update Feature Overview Header · UX brief

Companion of [spec.md](spec.md). The user's brief fixes what goes on each row; this note fixes the layout rules.

## Layout

```
┌──────────────────────────────────────────────────────────────────────┐
│ my-feature-slug  [Executing]              ◷ Publishing…  ⏸ ⏹ 🗑 ⋯ │  row 1
│ Squad v3 ·  orch/my-feature-slug/integration · started 2h ago       │  row 2
└──────────────────────────────────────────────────────────────────────┘
```

- `<header class="fh">` becomes a column of two rows, row gap `--space-xxs` (tight: row 2 belongs to the title).
- **Row 1**: flex row, `align-items: center`, `gap: --space-sm`. Slug (`--text-heading`, primary) · phase pill · spacer · optional Publishing indicator · `.head-acts`.
- **Row 2**: single line, `--text-caption`, `--color-text-muted` (`.muted`, as today), left edge aligned with the slug. Content and separators unchanged: `<team> v<n> · <branch icon> <branch> · started <ago>`; only the pieces present are joined (no leading ` · ` without a team).

## Width pressure

1. Row 2 ellipsizes at its end first (one line, never wraps).
2. Row 1: the slug ellipsizes first (`min-width: 0`, full slug in a tooltip).
3. While publishing on a narrow pane, the `Publishing N docs…` text also shrinks with an ellipsis, down to the `◷` glyph alone (the glyph always stays; full text in a tooltip). The slug has no fixed minimum width (a short slug is never padded); it shrinks first, with a much higher shrink factor than the Publishing text.
4. The pill and the actions never shrink or wrap; nothing overflows the pane.

## States

- **Running**: as drawn.
- **No run yet / finished run**: no pill when there is no phase; no `started` part when there is no run; no Pause/Stop buttons when finished (as today).
- **No team**: row 2 starts with the branch.
- **Publishing**: indicator on row 1 left of the actions, as today.

## Behaviour

No new keys, no new interactions. Tooltips, aria-labels and focus order of the actions are unchanged; tab order goes slug-row actions then the rest of the pane, as today.
