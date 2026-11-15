# Fix Double Lines on Command Launcher · Feature spec

> Feature `fix-double-lines-command-launcher` · Integration branch `orch/fix-double-lines-command-launcher/integration` · UI only (no core, no API change, no design work).

## 1. Problem

In the command launcher (⌘K command palette, and ⌘P jump, which share the same component), when a result does not fit on one line its text wraps onto a second line and draws over the next result. The list becomes hard to read.

## 2. Cause (HEAD 540838fe)

`ui/src/components/Palette.svelte`, `li` style: `display: flex; align-items: center; height: var(--size-row)` (28px). The row height is fixed, so when the `.label` (or a `.muted` part: `agent`, the disabled reason, the `detail` such as a project path) wraps, the extra line overflows the row and overlaps the row below. Long content comes from command labels, disabled reasons, `planHint()` details and jump targets (`detail: p.path`, pane titles), in a palette capped at `min(640px, 90vw)`.

## 3. Scope

A result row grows to fit its content instead of overflowing:

- The row keeps `--size-row` (28px) as its **minimum** height, so single-line rows look exactly as today.
- When the content wraps, the row becomes taller (vertical padding so wrapped lines don't touch the row edges or the selected highlight); the next row starts below it. No overlap, no clipping.
- The `.label` takes the free space and wraps; a long unbroken token (path, id) breaks rather than pushing the row wider than the palette (`overflow-wrap: anywhere`, `min-width: 0`).
- The muted parts (`agent`, disabled reason, `detail`) may wrap too, and keep their current order and colour.
- The key hint (`Kbd`) never shrinks or wraps and stays aligned with the row's first line (top-aligned when the row wraps). Single-line rows stay vertically centred as today.
- The selected-row highlight (`li.on`) covers the whole grown row; mouse hover and ↑/↓ selection behave as today.
- Applies to both modes (⌘K and ⌘P) since they share the component.

Out of scope: truncating/ellipsizing text (we show it all), the palette width, the list's max height, the ranking, the commands themselves, scrolling the keyboard-selected row into view.

## 4. Acceptance

1. In the running app, a palette result whose text needs two lines shows both lines inside its own row; the next result starts below; nothing overlaps (check ⌘K with a narrow window, and ⌘P with a long project path).
2. Single-line rows look unchanged (same 28px height, same alignment).
3. The key hint stays on the first line of a wrapped row and is never cut or wrapped.
4. The highlight of the selected row covers its full height.
5. A UI test guards the regression: the palette row uses a minimum height (no fixed `height`) and the label can wrap (e.g. a source/style assertion on `Palette.svelte`, plus a render test that a long label and detail are rendered in full in one `li`).
6. `bun run test`, `bun run check`, `bun run lint` pass in `ui/`.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| Palette rows grow with wrapped content + regression test | front | — | `ui/src/components/Palette.svelte` style only (markup only if needed for alignment), new/updated test in `ui/test/` |

`back`: no work (no data or API change). No design task: the fix restores the expected layout, no new visual.
