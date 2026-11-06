# Overwatch resize · design notes (T-1)

Source: Drive `Orchestrator.figcraft`, page **04.3 Chat column**, row at y = 14500:

| Board (FigCraft frame) | Export |
|---|---|
| Overwatch resize · 1512 · default 360 · handle at rest | `resize-default.png` (1×) |
| Overwatch resize · handle states (close-up) | `resize-states.png` (2×) |
| Overwatch resize · 1512 · widened 560 · mid-drag | `resize-wide.png` (1×) |
| Overwatch resize · 1280 · min 300 | `resize-min.png` (1×) |

## The handle

| Property | Value |
|---|---|
| Size | 4 px wide (`--space-xs`), full height of the Overwatch column (from under the title bar / banner to above the status bar) |
| Position | Straddles the column's existing 1 px left border (`--color-border-subtle` #3b4261): handle `left = border − 1 px`, so 1 px over the work column, 2 px over the Overwatch. The border itself never moves or thickens. Stacking: the handle must paint above the Overwatch (`.chat` is `position: relative`) and above the Inspector overlay (`z-index: 20`), so it stays visible, focus ring included, and grabbable while the overlay is open: e.g. `z-index: 21`, outside the `inert` `.chatwrap`. |
| Rest | No fill (`background: transparent`). Only the 1 px border shows. |
| Hover | `background: var(--color-border-strong)` (#545c7e dark / #8990b3 light), on plain `:hover` with no delay or transition, like the dock handle |
| Focus-visible | Same fill + the global focus ring: `outline: var(--border-width-focus) solid var(--color-focus)` (2 px, #7aa2f7 dark / #2e7de9 light), `outline-offset: 1px`. That is the app's `:focus-visible` rule, so nothing extra is needed beyond not suppressing it. |
| Dragging | Fill stays `--color-border-strong` (kept the same as the dock handle; no accent colour, so the dock needs no change). |
| Cursor | `col-resize` on the handle; during the drag `col-resize` on the whole window (e.g. on `body` or a full-window drag shield) and `user-select: none`. |
| Extras | No grip dots, no custom tooltip, no width read-out; only the native `title="Drag to resize · double-click to reset"`. |
| A11y | `role="separator"`, `aria-orientation="vertical"`, `aria-label="Resize Overwatch"`, `aria-valuenow` = width in px, `aria-valuemin="300"`, `aria-valuemax` = current max (the clamped bound, not a fixed 720; 720 is only the cap), `tabindex="0"`. |

Same CSS shape as `Dock.svelte`'s `.handle` (`.handle:hover, .handle:focus-visible { background: var(--color-border-strong) }`), plus a `.dragging` class with the same background.

## Widths drawn

Widths as in the app (`box-sizing: border-box`): the sidebar's / rail's 1 px right border and the Overwatch's 1 px left border sit inside their widths.

| Board | Window | Left | Work column | Overwatch |
|---|---|---|---|---|
| default | 1512 | sidebar 240 | 912 | **360** (`--size-chat`) |
| mid-drag | 1512 | sidebar 240 | 712 (≥ `--size-centre-min` 640) | **560** |
| min | 1280 | icon rail 48 | 932 | **300** (min) |

At 1512 with the sidebar the max that keeps the work column ≥ 640 is 1512 − 240 − 640 = 632 px, under the 720 cap.

## Column content at other widths

- Everything in the Overwatch that spans the column (header, digest, cards, rules, input/footer) stretches with it; right-aligned bits (Today ▾, search button, timestamps, "2 min", `c`, `⌘L search`) stay pinned to the right edge with their usual 12 px inset.
- At 300 px lines that no longer fit truncate with `…` (collapsed lines, outcomes) and card action rows wrap (Always allow moves to the "Open pane" row). No new layout rule: this is what the existing flex/ellipsis CSS already does at that width.
- The work column reflows with the drag; in the 560 board the Mission-control queue rows drop their waiting-on chips for lack of room (in the app they simply ellipsize).

Note: the boards are copies of the P4.3 shell boards, with the header renamed "Overwatch" and the footer set to the input placeholder "Ask the Overwatch…". Only the width and the handle are what this design specifies; the column content is the existing P4.3/P4.4 design.
