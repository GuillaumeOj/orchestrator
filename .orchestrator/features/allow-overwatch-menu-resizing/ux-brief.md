# Allow Overwatch Menu Resizing · UX brief

## Where

The shell (FigCraft page with the Overwatch / chat column boards, P4.4 · 04.3). The handle sits on the **left edge** of the Overwatch column, between the work column (panes + dock) and the Overwatch, full height from under the title bar / banner to above the status bar. Draw at 1512 and 1280 wide.

## The handle

- Hit area 4 px wide (`--space-xs`), straddling the column's existing left border; the border itself does not move or thicken at rest.
- **Rest**: invisible apart from the existing 1 px border; cursor `col-resize` on hover.
- **Hover** (plain `:hover`, no delay) and **focus-visible**: the 4 px strip fills with `--color-border-strong` (same as the dock's top handle). Focus also shows the standard focus ring token if the strip alone is not enough contrast.
- **Dragging**: the strip stays filled (`--color-border-strong`, or `--color-accent-default` if ui judges it clearer, as long as the dock handle gets the same treatment later); cursor `col-resize` everywhere on the window during the drag; no text selection.
- No grip dots, no tooltip at rest. A `title` on the handle: "Drag to resize · double-click to reset".

## Behaviour

- Drag left = wider, right = narrower. Live.
- Bounds: min 300 px, max 720 px, and never leaves the centre less than 640 px. At a bound the column simply stops (no bounce, no toast).
- Double-click (or `⏎` when focused) resets to the default width (360 at ≥ 1512, 340 below).
- Keyboard when focused: `←` wider / `→` narrower by 16 px, with `⇧` by 64 px; `Home` min, `End` max. Screen readers: separator "Resize Overwatch", value = current width in px.
- The chosen width is remembered across restarts and at every window size; a smaller window shrinks the column temporarily, a larger one gives the chosen width back.
- The Inspector overlay (below 1728 px) covers the column at whatever width it has.

## Boards to draw (ui)

1. The shell at 1512 with the Overwatch at its default 360 px, handle at rest, plus a close-up of the handle at rest / hover / focus / dragging.
2. The shell at 1512 with the Overwatch widened (e.g. 560 px), mid-drag.
3. The shell at 1280 with the Overwatch at its min 300 px.

Export them as PNGs to `.orchestrator/features/allow-overwatch-menu-resizing/design/` (e.g. `resize-default.png`, `resize-states.png`, `resize-wide.png`, `resize-min.png`).
