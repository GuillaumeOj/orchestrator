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
├─ row 1             flex row, align-items center, gap --space-sm (8px), min-height --size-control (28px)
│  ├─ .slug          shrinks with ellipsis, full slug in a use:tooltip (as the pill's)
│  ├─ .pill          (only with a phase) flex none
│  ├─ .grow          flex 1
│  ├─ ◷ Publishing…  (only while publishing) flex none, white-space nowrap
│  └─ .head-acts     flex none, unchanged (Pause/Resume, Stop, Delete feature…, ⋯)
└─ row 2             one line: overflow hidden, white-space nowrap, text-overflow ellipsis
```

Today `.fh` shares one flex-row rule with `.cardh, .tt, .ev, .msg, .colh` (`FeaturePane.svelte` style block): take `.fh` out of that list rather than changing the shared rule.

## Tokens

| Piece | Font | Colour | Other |
|---|---|---|---|
| `.slug` | `--text-heading` (600 18px/1.33) | `--color-text-primary` | `min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap` |
| `.pill` | `--text-label` (500 12px/1.33), inherited from `.feature` as today | border + text `runStateColor(phase)` (Executing = `--color-status-working`) | unchanged: padding `--space-xxs --space-sm`, `--border-width-default`, `--radius-sm`; `flex: none` |
| Publishing indicator | `--text-label`, inherited as today | `--color-text-muted` | `flex: none` |
| `.head-acts` | — | — | unchanged; `flex: none` |
| Row 2 (meta) | `--text-caption` (400 11px/1.45) | `--color-text-muted` (`.muted`, as today) | left edge = slug's left edge (no indent); same pieces and wording as today, joined by ` · ` between the pieces present (no team → row 2 starts with the branch glyph, no leading ` · `; today's markup emits one), branch glyph `\u{E725}` |

- Row gap between row 1 and row 2: `--space-xxs` (2px). The header's distance to the Notion line / columns below stays the pane's `--space-md` gap.
- Row 2 always has the branch piece (`orch/<slug>/integration` or `—`), so it is always rendered; the spec's "not rendered when empty" rule never fires today.

## Width pressure (state D)

Row 2 ellipsizes at its end. On row 1 only the slug shrinks (ellipsis); pill, Publishing indicator and actions keep their width and never wrap.
