# Allow multiple PRs · design exports

Final UI frames, FigCraft page **"allow-multiple-prs"** in the Drive `Orchestrator.figcraft` (one board per file below). Built on [ux-brief.md](../ux-brief.md) as updated in 2a79908a (new per-issue helper copy, mode lock copy, mission summary precedence) and [ux-flows.md](../ux-flows.md). The `ux-*.png` files are the UX wireframes, not final.

Existing tokens only (`ui/src/styles/tokens.css`): surfaces chrome / pane / raised / selected, border subtle / strong, text primary / secondary / muted / disabled, accent, status error / done / waiting-for-you, decision-gate-pr. Font: FiraCode Nerd Font Mono.

| File | ux-brief | What it shows |
|---|---|---|
| `launch-per-issue.png` | §1 | The launch dialog with two issues and One PR per issue: Issues and Pull requests sit right after the Brief textarea (before the intake note, the `Creates …` note and Run off-hours). The other fields are schematic: keep the dialog's own (Slug / Base row, `Brief (3 lines)` label, notes). |
| `launch-form-states.png` | §1, §8.1 | 1 no issue (per-issue disabled + tooltip `Add at least 2 issues`) · 2 two issues + One PR · 3 two issues + per issue · 4 invalid chip (Launch disabled) · 5 suggestions open · 6 one issue, per-issue disabled · 7 ten issues cap · 8 prefilled from an intake item. |
| `feature-header-issues.png` | §2.1 (ux-flows decision 1) | Header row 3: per issue, One PR with issues, no issue (muted `Add issues` link, `+` glyph in front), run done (no Edit), narrow 420 px truncation, nothing when no issue and done. |
| `feature-prs-card.png` | §2.2, §8.2 | per_issue card rows (none, pending, open with CI error + behind, merged, closed) at 320 px (state on line 2) and 560 px (same line), single mode unchanged, min width 260 px, truncation order of an open row + ifCut tooltip. |
| `edit-popover.png` | §2.1, §8.3 | Popover: editable · mode locked (`Set before the first PR: this feature already has one.`) · issue locked (lock glyph, no ×, `Has PR #34`) · saved during a run · server error inline. |
| `decision-cards.png` | §3, §8.4 | push_pr per issue (two gates side by side) · push_pr_update per issue + force variant · single mode with `Closes #12, ENG-7`. |
| `mission-run-row.png` | §4, §8.5 | Run row `2 PRs open · 1 merged` with red dot and per-PR tooltip · summary copy by precedence (dot colours) · run inspector compact rows. |
| `board-issue-chip.png` | §5, §8.6 | Task cards with the `#12` chip after the model chip, untagged task without chip, long ref on a narrow card, task inspector header (truncation handle > agent > chip). |

Notes for front:
- The issue chip in the chip input is surface-raised + border-subtle, radius sm; invalid = status-error border on a 15 % status-error tint (`color-mix(in srgb, var(--color-status-error) 15%, transparent)`, as DiffPane / Menu do; not the syntax token, which differs in light theme); focus = 2 px focus ring.
- Segmented control = the existing `.seg` (Dialogs.svelte). It has no disabled style nor radio semantics yet: add `.seg button:disabled` (text-disabled, no pointer), `role="radio"` + `aria-checked` and arrow keys (ux-brief §7). The disabled reason stays reachable as a tooltip, and in the popover the mode lock is also a muted inline line with the lock glyph.
- PR card rows: the open state is PrLine's content (`PR #34`, CI dot + count, position) with the ref in front, but without PrLine's own checked-ago and refresh (one refresh and `checked 1m ago`, from `checkedText`, on the card head). Row truncation follows ux-brief §2.2 (title, then position down to `↓3`, then CI count), not PrLine's §1.6 order.
- Pending row colour = status-waiting-for-you; the board/inspector issue chip reuses the TaskModelChip style.
- Sample data in the frames is illustrative (issue numbers and agent nodes vary between frames); only the layout, copy and colours are normative.
