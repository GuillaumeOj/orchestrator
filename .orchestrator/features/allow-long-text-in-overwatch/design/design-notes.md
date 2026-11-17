# Allow Long Text In Overwatch · Design notes

> From [ux-brief.md](../ux-brief.md) and [spec.md](../spec.md) §3.2–3.3. **Final for front and ui**: copy and visuals here win over the brief and the spec; behaviour changes are in "Changes to the spec" (lead settles).
> Read against **main**: `ChatInput.svelte` (a mono `textarea`, `.hints` row with the status hint and the counter, a `.warn` `role="alert"` line for the over-length), `ChatText.svelte` (user `.body.plain` on `--color-accent-subtle`, mention links as inline `span role="link"`), `ChatColumn.svelte` (`unsent` entries with Retry, the `.sr` live region), `ui/src/styles/tokens.css`. No new token, no new colour. Everything is the app's mono face. Names of mention notes' components are reused where they fit.

## 1. In the input

**Placeholder text (final).** `[Pasted text #<n> · <chars> chars]`, as the contract (`3 214` with plain-space grouping, `spaced()`). **No line count in the field**: the longest form `[Pasted text #10 · 200 000 chars]` is 33 mono characters ≈ 257 px at 13 px, which fits the 300 px column's field on one line (≈ 276 px inner); adding `· 120 lines` would wrap at 300. The line count lives in the chip (§2), where it is useful.

**Confirmation caption: dropped.** The placeholder appearing at the caret is the confirmation; a caption would add a timed state, a second place to look and a claim ("Backspace removes it") the user finds out in one keystroke. Nothing is added to the `.hints` row. Screen readers get the live-region line (§5). Board 1 therefore shows the field only.

**Removal and undo.** Backspace just after / Delete just before a placeholder removes it whole (spec §3.2), through the textarea's own editing (`execCommand('delete')`) so **⌘Z restores the placeholder, and the paste with it**: front keeps the `app.chatPastes` entry until send (unreferenced ones are dropped at send, as the spec says). No undo hint: ⌘Z is standard and a removed paste is one keystroke from back.

**Errors**: in the `.warn` slot (error colour, `--text-caption`, `role="alert"`), in the place of the over-length warning, nothing else moves.

| Case | Copy (final) | Behaviour |
|---|---|---|
| Paste over 200 000 chars | `Not pasted: 312 450 characters, at most 200 000 in a pasted text.` (the real count, grouped) | nothing is inserted, the draft is unchanged |
| 11th long paste | `Not pasted: at most 10 pasted texts in one message.` | same. A *short* paste (inline) is never refused by this limit |
| Typed text over 2 000 (new) | `Too long for a message. Paste long text instead: it is sent as a file.` | as today: send blocked, `aria-invalid` |

A paste error stays until the next keystroke, paste, or send; then the over-length line (if it applies) shows again. If both apply the paste error shows first. Not auto-dismissed on a timer (a `role="alert"` that vanishes unread is worse than one that lingers).

**Counter.** Unchanged: it shows from 1 800 and counts placeholders as their text (33 chars each at most).

**With the planning tag and a mention** (board 2): the tag stays above the field, the mention popup stays anchored to the input block as in the mention notes; a placeholder in the field changes nothing there. `@` is never completed inside a placeholder.

## 2. The paste chip in a sent entry

In `ChatText.svelte`, user `.body.plain`: each `[Pasted text #n …]` whose `n` is in `meta.pastes` (or the context, for unsent / waking) is a chip. The token's text is replaced by the chip; the text around it is untouched. Tokens with no matching paste stay plain text.

**Element.** `<span class="paste" role="button" tabindex="0" aria-expanded aria-controls>` (a span, like the mention link: it stays selectable in a copied message). The template has no whitespace between its parts (`white-space: pre-wrap` body). `display: inline-block; max-width: 100%; vertical-align: baseline; white-space: nowrap; overflow: hidden; text-overflow: ellipsis` (a chip never wraps across lines; ellipsis only as a last resort). `font: inherit; line-height: inherit`: **no line-height jump**, so the ring is `box-shadow: inset`, not a border. `padding: 0 var(--space-xs)`, `border-radius: var(--radius-sm)`, `cursor: pointer`.

**Content**: a 12 px document glyph (inline SVG, `currentColor`, `aria-hidden`, `margin-right: var(--space-xs)`) + `Pasted text #1 · 5 000 chars · 120 lines`. Regular weight (a mention is semibold).

**Look, distinct from a mention.** A mention is a **filled** semibold pill; the chip is an **outlined** regular-weight element with a glyph.
- **Rest**: `background: transparent`; `box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-accent-default) 50%, transparent)`; `color: var(--color-text-primary)`.
- **Hover**: `background: color-mix(in srgb, var(--color-accent-default) 22%, var(--color-accent-subtle))`, ring `--color-accent-default`. Tooltip (`lib/tooltip.js`): `Show pasted text` / `Hide pasted text`.
- **Focus-visible**: `box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus)` (as `.text:focus-visible`), kept over the ring.
- **Open** (`aria-expanded="true"`): the hover fill stays, ring `--color-accent-default`: it reads as attached to its panel.
- **Missing file** (known once the fetch said `not_found`, §3): ring dashed (`outline: 1px dashed`-style, via a repeating-gradient or `outline` at `-1px` offset), `color: var(--color-text-secondary)`, no fill on hover, glyph replaced by a struck-through-document variant. Still focusable and expandable (the panel says why). Tooltip `Pasted text no longer available`.
- ui checks WCAG 2 contrast of the text and ring on the wash in both themes and records the fixes below, as for the mention.

**Narrow.** The chip's own container (the bubble) under **300 px of content width** drops ` · 120 lines`, keeping `Pasted text #1 · 5 000 chars`; the full label stays in the accessible name and the panel header. At 300 px column that is the usual case.

**Keys.** ⇥ reaches it in reading order. ⏎ or ⎵ toggles (⎵ `preventDefault`s the scroll); click toggles.

## 3. The expansion

**Where.** A panel **under the bubble**, not inside it: the bubble stays the user's own words, the panel is a different kind of content. Same width as the bubble (right-aligned with it like the bubble), `margin-top: var(--space-xs)`, before the delivery marker / timestamp line. Surface `--color-surface-raised`, `1px --color-border-subtle`, `--radius-md`, no shadow.

**Many open.** Each chip toggles its own panel; any number can be open (bounded: each is max 16 lines tall and scrolls). Several panels of one entry stack under the bubble in the order of `n`, regardless of the click order. State is per entry + `n`, lives in memory, resets on reload (collapsed).

**Header** (a 28 px row, `--text-caption`, `--color-text-muted`, bottom `--color-border-subtle`, left padding `--space-sm`): `#1 · 5 000 chars · 120 lines` at the left (always the full numbers, even when the chip dropped the lines). At the right, two icon buttons (`--size-control-sm`): **Copy** (label `Copy`, text button, `--text-caption`; becomes `Copied` for 1.5 s, announced) and **Collapse** (`✕`, `aria-label="Hide pasted text 1"`, tooltip `Hide`).

**Body.** The text in `--font-family-mono` `--text-mono-sm`, `--color-text-primary`, `white-space: pre-wrap; overflow-wrap: anywhere` (no horizontal scroll), `padding: var(--space-sm)`. **Max height 16 lines** (`16 × the line-height`, plus the padding), then `overflow-y: auto`, `overscroll-behavior: contain`. The scroll box is `tabindex="0"` with `role="region"` `aria-label="Pasted text 1"` so the keyboard scrolls it (⎵, ↑↓, ⇞⇟), focus ring as above. The text is a single text node (a 200 000-character paste must not make a node per line). **Plain text only: no Markdown, no syntax colours.**

**Copy** copies the full pasted text (not the visible part) with `navigator.clipboard.writeText`; on failure the button says `Copy failed` for 1.5 s.

**Collapse.** The ✕, the chip again, or **Esc** with focus inside the panel; Esc and ✕ return focus to the chip. Esc with focus elsewhere does what it does today.

**States.**
| State | Panel |
|---|---|
| Loading (first open of a sent entry; `chat_paste` in flight) | header as above (Copy disabled), body one muted line `Loading pasted text…`, `min-height` of a line, `aria-busy="true"`. No spinner: it is a local file, usually under a frame |
| Loaded | the text; kept for the session, a later open is instant |
| Missing (`not_found`) | header without Copy, body muted line `Pasted text no longer available` (`role="status"`); the chip goes to its missing look |
| Other error | body `Could not load pasted text: <error> · Retry` (the `.link` Retry the column already uses); the chip is unchanged |
| Unsent / waking entry | opens at once from the pastes in memory, no loading state |

## 4. Unsent entry

Same bubble and chips as a sent one, panels openable from memory; the existing `unsent` line and **Retry** under it, unchanged. Retry sends the same pastes. The chip label uses `chars` and `lines` computed from the paste in memory.

## 5. Keys and screen reader

**Keys (input)**

| Key | Does |
|---|---|
| paste (⌘V) | long plain text: a placeholder; otherwise inline as today |
| ⌫ just after a placeholder, ⌦ just before | removes it whole; ⌘Z restores |
| ⌥⌫ just after a placeholder | same as ⌫ (see changes, 2) |
| ⌘Z / ⇧⌘Z | undo / redo the insert or the removal, paste included |
| ⏎, ⇧⏎, Esc, ⌘L | unchanged |

**Keys (thread)**: chip ⏎ / ⎵ toggles; inside the panel ⇥ goes Copy → ✕ → the scroll box; Esc collapses.

**Screen reader**
- Chip: `role="button"`, `aria-expanded`, `aria-controls` the panel's id, name `Pasted text 1, 5 000 characters, 120 lines` (`, no longer available` appended when missing). The glyph is `aria-hidden`.
- Panel: `role="region"`, `aria-label="Pasted text 1"`; the scroll box is the region itself.
- Live region (`.sr` in `ChatColumn`, via `onannounce`): after a placeholder is inserted `Pasted 5 000 characters as pasted text 1`; after a whole removal `Removed pasted text 1`; after Copy `Copied pasted text 1`. Errors are `role="alert"`, not repeated in the live region. Opening / closing is carried by `aria-expanded`.
- Textarea: unchanged (`aria-describedby="ow-input-hint"`); the placeholder is plain text and is read as such.

## 6. Copy (final; every string)

| Where | String |
|---|---|
| Placeholder (in the draft) | `[Pasted text #<n> · <chars> chars]` |
| Chip | `Pasted text #<n> · <chars> chars · <lines> lines` (`· <lines> lines` dropped when narrow; `1 line` singular) |
| Chip tooltip | `Show pasted text` · `Hide pasted text` · `Pasted text no longer available` |
| Chip name | `Pasted text <n>, <chars> characters, <lines> lines` |
| Panel header | `#<n> · <chars> chars · <lines> lines` · `Copy` · `Copied` · `Copy failed` · `✕` (`Hide pasted text <n>`) |
| Loading | `Loading pasted text…` |
| Missing | `Pasted text no longer available` |
| Load error | `Could not load pasted text: <error> · Retry` |
| Error, too big | `Not pasted: <count> characters, at most 200 000 in a pasted text.` |
| Error, 11th | `Not pasted: at most 10 pasted texts in one message.` |
| Error, over-length | `Too long for a message. Paste long text instead: it is sent as a file.` |
| Live region | `Pasted <chars> characters as pasted text <n>` · `Removed pasted text <n>` · `Copied pasted text <n>` |
| Input placeholder | unchanged: `Message the Overwatch · ⏎ to send` |

## 7. Frames for ui (names kept)

Drive `Orchestrator.figcraft`, P4.4 · 04.3 chat column page, next to the mention boards (row below `mention-*`). Tokyo Night dark (light variant: ui's call, same tokens), exported under `design/` as PNG. Column frame 360 × 800 unless said; thread above with real entries, input block at the foot.

| File | Shows |
|---|---|
| `paste-input-after.png` | brief board 1: two states side by side, 360 px: empty field → after a 5 000-character paste (`[Pasted text #1 · 5 000 chars]` at the caret, field one line high). **No caption.** |
| `paste-input-mixed.png` | board 2, 360 px: planning tag, typed text + two placeholders (#1, #2) + a `@mention` token, counter not shown; the caret after #2 |
| `paste-input-errors.png` | board 3: three columns, 360 px: the three error lines of §1 (too big with a real count, 11th, over-length), the field in the matching state |
| `paste-chip-states.png` | board 5 (states): one entry per state, 360 px: chip at rest, hover (with tooltip), focus-visible, open, missing; beside one mention link at rest to show the difference |
| `paste-expanded.png` | board 5: entry with one chip open, 360 px: header (`#1 · 5 000 chars · 120 lines`, Copy, ✕), 16 lines of a stack trace with the scrollbar, nothing below cut |
| `paste-multi.png` | board 6: one entry with two chips, #1 open and #2 closed; the panel under the bubble, above the time line |
| `paste-loading-missing.png` | board 6: three panels: loading, missing (muted chip), load error with Retry |
| `paste-unsent.png` | board 7: unsent entry with a chip and its Retry; panel closed |
| `paste-narrow.png` | board 8, 300 px: `paste-input-mixed` and `paste-expanded` side by side; the chip without its lines, the 33-character placeholder on one line |

Hand the exports to front under `design/` (PNG); a link-only hand-off is not enough.

## Changes to the spec (lead decides)

The thresholds (800 characters or 10 lines), the placeholder format, the Backspace / Delete rule, the 10-paste and 200 000-character limits all read right in use and are kept. Behaviour details the spec left open or implies differently:
1. **No confirmation caption** (brief §2 left it to ux). Copy-level; nothing for front to build.
2. **⌥⌫ just after a placeholder removes it whole**, like ⌫ (native ⌥⌫ would delete `chars]` and silently turn it into plain text that is no longer sent as a paste). Other deletions inside a placeholder are native, as the spec says.
3. **The 10-paste limit counts placeholders still in the draft**, not pastes made: a removed one frees its slot. `n` stays "never reused within a draft" (highest `n` so far + 1); past 99 (unreachable in practice) front shows the 11th-paste error.
4. **Limit errors refuse only long pastes**; a paste under the thresholds is always inserted inline, whatever the count.
5. **Many panels open** at once (spec: "one or many" open question), each per entry + `n`, collapsed on reload.
6. **The chip drops ` · <lines> lines` under 300 px of bubble width**; the full label stays in its accessible name and the panel header.
7. **The chip mutes on a missing file only after the fetch says so** (the file is checked on first open: no IPC on render).
