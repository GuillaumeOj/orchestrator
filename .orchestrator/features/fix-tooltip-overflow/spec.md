# Fix Tooltip Overflow · Feature spec

> Feature `fix-tooltip-overflow` · Integration branch `orch/fix-tooltip-overflow/integration` · UI only (no core, no API change, no design file change).

## 1. Problem

When a tooltip's text is long, it runs out of the tooltip's box: the text keeps going on one line past the border instead of staying inside it.

## 2. Today (HEAD 540838fe)

One shared tooltip element on `document.body`, built by `ui/src/lib/tooltip.js` (`use:tooltip={{ label, key?, detail? }}`, design-notes §2.5 of `improve-orchestrator-ui-ux`), styled by `.tooltip` in `ui/src/styles/app.css:120-155`:

- `.tooltip` has `max-width: 480px` **and** `white-space: nowrap`: the box stops at 480 px but the text never breaks, so a longer label overflows the box.
- `.tip-line` is a flex row: label text, then (when there is a key) a muted `·` and a `.kbd`.
- `.tip-detail` (second line, caption, muted) has `overflow: hidden; text-overflow: ellipsis` but no width of its own, so it overflows like the label.
- `min-height: 24px`; the height is otherwise set by the content. Placement (`placeTip`) already uses the measured `offsetWidth` / `offsetHeight` after filling, from the top-left corner.

## 3. Scope

A tooltip whose text does not fit wraps onto as many lines as it needs, and the box grows in height to fit them. No fixed height.

1. **Wrapping.** Label and detail wrap at word boundaries when they reach the tooltip's maximum width. A long token with no spaces (a path, a URL, a branch name) breaks anywhere rather than overflow (`overflow-wrap: anywhere`).
2. **Width.** Maximum width stays 480 px, and is also capped to the window width minus the 4 px edge margin on each side, so a tooltip never goes past the window edge on a narrow window. A short tooltip stays as narrow as its text (no change).
3. **Height.** No fixed height: the box is as tall as its lines plus the current padding. `min-height: 24px` stays, so a one-line tooltip looks exactly as today.
4. **Key hint.** With a `key`, the `·` and the `.kbd` never wrap or shrink: they stay together at the end of the label, vertically centred on the label block. Only the label text wraps.
5. **Detail.** The detail line wraps the same way; no ellipsis any more (the tooltip exists to show the full value).
6. **Placement.** Unchanged rules (below with 6 px gap, flipped above at the bottom, kept inside the window), using the wrapped size: measured after the text is laid out at its final width, so a multi-line tooltip flips above and stays inside the window correctly.

Out of scope: tooltips taller than the window (none are that long), honouring `\n` in tooltip strings, native `title=` tooltips, the Select/Menu popover, the Figcraft design file (`02-tooltip.png` shows one-line tooltips, which do not change).

## 4. Acceptance

1. In the running app, hovering a control whose tooltip text is longer than 480 px shows all the text inside the box, on several lines; the border and background enclose every line.
2. A one-line tooltip (e.g. "Hide dock · ⌘J") looks identical to today (same height, width, padding).
3. A long label with a key: the label wraps, `· key` stays together on the right and is not split.
4. A long path in `detail` (no spaces) wraps inside the box.
5. A multi-line tooltip near the bottom of the window flips above the control without covering it; near the left/right edge it stays inside the window.
6. Tests: `ui/test/menu.test.js` `tooltip` block gains a case for the wrapping structure (label in its own element, dot + key apart from it) and `placeTip` with a multi-line height; existing tooltip tests pass unchanged in intent. `bun run test`, `bun run check`, `bun run lint` pass in `ui/`.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| Wrap long tooltips + tests | front | — | `ui/src/styles/app.css` (`.tooltip`, `.tip-line`, `.tip-detail`), `ui/src/lib/tooltip.js` (wrap the label text in its own element if the CSS needs it), `ui/test/menu.test.js`. Checks it in the running app at a long label, a long detail and a narrow window. Asks `back` for the code review. |

`back`: no work (no data change); reviews front's branch.
