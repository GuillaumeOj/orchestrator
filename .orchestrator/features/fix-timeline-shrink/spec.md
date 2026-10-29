# Fix Timeline Shrink · Feature spec

> Feature `fix-timeline-shrink` · Integration branch `orch/fix-timeline-shrink/integration` · UI only (no core, no API change, no design pass)

## 1. Problem

In the Feature pane's **Timeline** card, a long event text wraps onto several lines (expected). On those rows the time's AM/PM suffix also wraps under the hour ("10:42" / "AM"). On one-line rows the time stays on one line.

**Goal:** the time of a timeline row always shows on one line ("10:42 AM"), whatever the length of the event text.

## 2. Cause (HEAD 44599b6)

`ui/src/panes/FeaturePane.svelte:275`: each row is a flex row (`.ev`, `display: flex; align-items: flex-start`) of three items: the time `<span class="mono muted">` (from `toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })`), a compact `AgentChip`, and the text `<span class="evt">`.

Flex items default to `flex-shrink: 1; min-width: auto`. When the text is long, the row overflows and the browser shrinks every item down to its min-content width; the time's min-content width is its longest word ("10:42"), so the line breaks at the space before "AM"/"PM". With 24-hour locales the bug is invisible (no space), which is why it looks random.

## 3. Scope

- The timeline time never shrinks and never wraps: it keeps its natural width (`flex: none` + `white-space: nowrap`) through a named class on the time span (e.g. `.evtime`), styled next to `.ev` / `.evt` in the same component.
- The compact `AgentChip` in the row must not shrink either (it is already one fixed-width mark; make sure it stays aligned with the first line).
- The event text (`.evt`) takes the remaining width and wraps as today (`min-width: 0`, it may break long words so it never pushes the row wider than the card).
- No change to the time format, to the locale, or to any other list.

Out of scope: other time displays (`ago(...)` labels are not affected), timeline filters, the message box.

## 4. Acceptance

1. A timeline row with a long multi-line text shows its time on one line, top-aligned with the first line of the text, in a 12-hour locale ("10:42 AM").
2. One-line rows look exactly as today.
3. A long unbroken token in the text (a path, a URL) does not widen the card or push the time onto two lines.
4. A component test (in `ui/test/teams.test.js`, which already renders `FeaturePane`) asserts the time span carries the non-shrinking class; `bun run test`, `bun run check`, `bun run lint` pass.

## 5. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Timeline time stays on one line | front | `FeaturePane.svelte` markup + style, test in `teams.test.js` |

No backend work: `back` reviews the frontend change.
