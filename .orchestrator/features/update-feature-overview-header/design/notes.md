# Update Feature Overview Header · design notes (T-344, ui)

FigCraft file `Orchestrator.figcraft` (Google Drive, not in the repo), page **04.4 Feature & board**:

| Export | Layer | Shows |
|---|---|---|
| `feature-header.png` (2x) | `04 · Feature pane after the cleanup …`, layer `fh · Feature header` | Running state in the full screen |
| `feature-header-states.png` (2x) | `Feature header · states (update-feature-overview-header)` (x 5520, y 120) | A running · B no run · C publishing · D narrow 420 px pane · E no team, no run |

Layout and copy follow [../ux-brief.md](../ux-brief.md). Existing tokens only, no new token.

## Structure

```
header.fh            column, gap --space-xxs (2px)
├─ row 1             flex row, align-items center, gap --space-sm (8px), height --size-control (28px)
│  ├─ .slug          min-width 0, ellipsis, title = full slug
│  ├─ .pill          (only with a phase) flex none
│  ├─ .grow          flex 1
│  ├─ ◷ Publishing…  (only while publishing) flex none, white-space nowrap
│  └─ .head-acts     flex none, unchanged (Pause/Resume, Stop, Delete feature…, ⋯)
└─ row 2             one line: overflow hidden, white-space nowrap, text-overflow ellipsis
```

## Tokens

| Piece | Font | Colour | Other |
|---|---|---|---|
| `.slug` | `--text-heading` (600 18px/1.33) | `--color-text-primary` | `min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap` |
| `.pill` | body (13px), as today | border + text `runStateColor(phase)` (Executing = `--color-status-working`) | unchanged: padding `--space-xxs --space-sm`, `--border-width-default`, `--radius-sm`; `flex: none` |
| Publishing indicator | body, as today | `--color-text-muted` | `flex: none` |
| `.head-acts` | — | — | unchanged; `flex: none` |
| Row 2 (meta) | `--text-caption` (400 11px/1.45) | `--color-text-muted` (`.muted`, as today) | left edge = slug's left edge (no indent); same text and ` · ` separators as today, branch glyph `\u{E725}` |

- Row gap between row 1 and row 2: `--space-xxs` (2px). The header's distance to the Notion line / columns below stays the pane's `--space-md` gap.
- Row 2 is not rendered when it would be empty.

## Width pressure (state D)

Row 2 ellipsizes at its end. On row 1 only the slug shrinks (ellipsis); pill, Publishing indicator and actions keep their width and never wrap.
