# Add Mention In Overwatch · Design notes

> From [ux-brief.md](../ux-brief.md) and [spec.md](../spec.md) §3.2–3.3. **Final for front and ui**: copy and visuals here win over the brief and the spec; behaviour changes are in "Changes to the spec" (lead settles).
> Read against **main**, not this branch: `ChatInput.svelte` (placeholder "Message the Overwatch · ⏎ to send", no key-hints row any more; a `.hints` line holds the status hint and the counter), `ChatText.svelte`, `ui/src/styles/tokens.css`, `Palette.svelte` (the ⌘P list), `lib/teams.js` (`runStateLabel`, `runStateColor`). No new token, no new colour. Everything is the app's mono face.

## 1. The popup

**Anchor and size.** A child of the input block (`.input`, made `position: relative`), `position: absolute; left: 0; right: 0; bottom: calc(100% + var(--border-width-default))`. So it is the input block's full width, i.e. the column width (no side margin: it spans the 12 px padding of the block), and its bottom edge sits on top of the input block's top border (a plain `bottom: 100%` is measured from the padding box and would cover that border). It overlays the thread; it never moves the thread, the input or the field's height.
- **360 px column**: popup 360 px wide. **300 px column**: 300 px wide. (Not the textarea's width: the extra 12 px each side buys the slug room.)
- **Anchor is the input block, not the textarea.** With the planning tag shown (P4.5, §5 state 8) the popup stays above the tag: the tag is never covered. When the field grows to 5 lines the popup keeps its bottom on the block, so it rises with it.
- Surface: `--color-surface-raised` (the ⌘P panel, `.pal`, uses `--color-surface-pane`; the popup is lifted one step because it sits on the pane-coloured thread), `1px --color-border-strong`, `--radius-md`, `--shadow-overlay` (as `.pal`), `z-index` above the feed and under the palette scrim (z 50). Inner padding `--space-xs`, like `.pal ul`.
- Height: up to 8 rows. List `max-height = min(8 × 32 px + the group headers (20 px each), 50vh)` (the 50vh cap is the ⌘P list's, so a short window never pushes the popup above the column); beyond it the list scrolls (`overflow-y: auto`). The footer is not part of the scrolling list. The highlighted row is kept in view with `scrollIntoView({block:'nearest'})`.

**Groups.** **Current** (a run whose status is not done / failed), then **Other features**. A group header is a 20 px caption line (`--text-caption`, `--color-text-muted`, left padding `--space-sm`): `Current`, `Other features`. **Headers are shown only when both groups have rows**: a lone group needs no title (a lone "Other features" header would suggest a missing "Current"). Headers are not options.

**Footer hint.** `↑↓ to choose · ⏎ to insert · esc to close`, a 24 px line under the list, `--text-caption`, `--color-text-muted`, top border `--color-border-subtle`, left padding `--space-sm`.
- **Kept at 360**, **dropped at 300** (it costs 24 px of height and nothing at 300 px is more useful than rows). Rule: container query on the popup, hint hidden when the popup is under 340 px wide. The same sentence stays available to screen readers (below).
- Not shown in the no-match / no-features states (nothing to choose).

**Behaviour that follows from the surface.** `mousedown` on the popup is `preventDefault`-ed: the textarea keeps focus (blur closes the popup, so a click would otherwise cancel itself). Opening highlights row 1. Mouse: `mousemove` (not `mouseenter`) sets the highlight, as ⌘P does, so a scroll under a still pointer doesn't steal it.

## 2. A row

Single line, `min-height 32px` (= `--space-2xl`; no row token exists between `--size-row` 28 and `--size-row-dense` 40; 32 is the brief's, and keeps 8 rows near 256 px), `padding 0 --space-sm`, `gap --space-sm`, `--radius-sm`, `align-items: center`. Same pattern as `Palette.svelte` `li`.

| Part | Spec |
|---|---|
| Slug | `@` + slug, `--font-family-mono` 13 px (`--text-mono`), `--color-text-secondary`. **Matched characters** (the fuzzy positions of the query in the slug): `--font-weight-semibold` and `--color-accent-default` (mono keeps the width). On the highlighted row the accent on `--color-surface-selected` is about 4.2:1: ui checks the contrast; if under 4.5, matched chars on the highlighted row go `--color-text-primary` + underline. When the match came from the title or project instead, the slug has no emphasis and the matched chars are emphasised in that field the same way. `flex: 0 1 auto`, `min-width: 8ch`, ellipsis (end) as the last resort. |
| Title | `--text-body`, `--color-text-muted`, `flex: 1 1 0; min-width: 0`, ellipsis. **Hidden at 300 px** (container query, popup < 340 px) : the slug and the project say which feature it is. |
| Project | `--text-caption`, `--color-text-muted`, `max-width 12ch`, ellipsis, `flex: none`. Never hidden: it tells two same-slug features apart. |
| Run state | `--text-caption`: a `--size-status-dot` (8 px) dot in `runStateColor(status)` + `runStateLabel(status)` ("Executing", "Spec gate", "PR open", "Stopped"…) in `--color-text-secondary`; `flex: none`, **never truncated**. No run: no dot, `No run` in `--color-text-muted`. (Not `StatusBadge`: it is the agent badge; the dot + label is the sidebar's run-state look.) |

**Highlighted row** (keyboard or hover): `background: var(--color-surface-selected)`, nothing else changes: the ⌘P `li.on`. No border, no left bar. The title stays `--color-text-muted`. Focus never leaves the textarea, so there is no focus ring in the list.

**At 300 px** the row reads `@slug … project · ● state` (title dropped); at 360 `@slug  title … project · ● state`, the title taking whatever the slug leaves, usually truncated to a few characters, which is fine.

## 3. States (ux-brief §4, settled)

1. **Open, empty query** (`@` just typed): current group, then other features, each by most recent activity; row 1 highlighted.
2. **Filtered** (`@ment`): rows by score, groups kept, matches emphasised, row 1 highlighted.
3. **No match**: one non-selectable row, `--color-text-muted`, `padding 0 --space-sm`, 32 px: `No feature matches “xyz”` (typographic quotes; the echo is cut at 24 characters with `…`). No footer, no headers. **⏎ and ⇥ close the popup and keep the text; nothing is sent, nothing inserted.** The popup stays closed until the token changes: **any edit of the token** (a character typed in it or deleted from it) or a new `@` typed reopens it with the new query. Moving the caret (arrows, click) does not reopen it. The same rule holds after **Esc** (§5): closed by Esc, reopened by the next edit of the token. A second ⏎ then sends as usual. Rationale: ⏎ is "send" for every other state of the field; swallowing it once, visibly (the popup disappears), is the cost of the rule "⏎ never sends while the popup is open", and the user keeps their text.
4. **No features at all** (none non-archived): **the popup opens**, so that `@` isn't silently dead: one non-selectable row `No features yet`. Same ⏎ / ⇥ behaviour as no match. It closes on the space that ends the token like any other.
5. **Same slug in two projects**: two rows, same slug; the project caption tells them apart; order by score, then project name A→Z. Each row carries its feature id: the pick remembers that id (spec §3.2), so the right one resolves.
6. **Sent message**: §4. 7. **Removed feature**: §4.
8. **Planning tag shown**: the popup sits **above the tag** (anchor = the input block, §1); the tag stays visible and in place. Closing the popup changes nothing below it.

## 4. The mention in a sent entry

In `ChatText.svelte`, the user's `.body.plain` (`--text-body`, on `--color-accent-subtle`). Each token matching `meta.mentions` is an inline element inside the pre-wrapped text; the `@` is part of it.

**Mention link (rest)**: `<span class="mention" role="link" tabindex="0">` (not a `<button>`: a button is always inline-block, so it would not wrap with the text, would ignore `box-decoration-break`, and its text is not selectable in WebKit, which would drop the mention from a copied message). The token is rendered with **no whitespace between segments** in the template (the body is `white-space: pre-wrap`, so any template newline or indent would show). `display: inline; user-select: text; font: inherit; line-height: inherit` (no line-height jump: the pill is horizontal padding only), `padding: 0 var(--space-xs)`, `border-radius: var(--radius-sm)`, `background: color-mix(in srgb, var(--color-accent-default) 22%, var(--color-accent-subtle))`, `color: var(--color-text-primary)`, `font-weight: var(--font-weight-semibold)`, `box-decoration-break: clone`, `overflow-wrap: anywhere`, `cursor: pointer`. It reads as a token on the wash and stays text-sized.
- **Hover**: background mix 38% accent, plus `1px` `--color-accent-default` underline (`text-decoration-thickness: 1px; text-underline-offset: 2px`). Tooltip (the app's `tooltip` action, `lib/tooltip.js`): `<title> · <project>`.
- **Focus-visible**: `box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus)`, as `.text:focus-visible`. Reachable with ⇥ in reading order. ⏎ / click → `openFeature(id)` (link semantics: ⎵ does nothing).
- **Accessible name**: `aria-label="Open feature @slug"`.

**Removed feature** (the id is absent from `app.data.features` once the data has loaded: until then, e.g. at start-up, every token renders as a live mention rather than flashing "Feature removed"; an archived feature is still in the list and stays a live mention, as the core keeps archived ids): not a link, a `<span class="mention gone">`: no background, no underline, `color: var(--color-text-muted)`, same font and padding-less (no pill), `cursor: default`. Tooltip `Feature removed`; a visually hidden ` (feature removed)` follows it for screen readers. Not focusable.

A token without a matching `meta.mentions` entry stays plain text, unchanged. Unsent / waking entries render the same from their context. The planning line (`planning · <project>`) is unaffected.

## 5. Keys (popup open; the field keeps focus)

| Key | Does |
|---|---|
| ↓ / ⌃N | next row (stops at the last; no wrap) |
| ↑ / ⌃P | previous row (stops at the first) |
| ⏎ / ⇥ | insert the highlighted row; in states 3 and 4 close the popup, keep the text |
| Esc | closes the popup only (`stopPropagation`); a second Esc leaves the field as today |
| click | inserts that row |
| ⇧⏎ | newline as today, popup closes (a newline ends the token) |
| ⇧⇥ | not intercepted: focus moves back as usual, the popup closes on blur |

⏎ is ignored while composing (`isComposing` / keyCode 229), as the existing handler does. Inserted text `@<slug> ` (trailing space, none added if one follows), caret after it, popup closes. The popup never opens from a click or paste, only from typing `@` (or caret moving into a token by typing/deleting inside it).

## 6. Copy (final; every string)

| Where | String |
|---|---|
| Group headers | `Current` · `Other features` |
| Row | `@<slug>` · `<title>` · `<project>` · `<state label>` or `No run` |
| No match | `No feature matches “<query>”` |
| No features | `No features yet` |
| Footer hint (360 only) | `↑↓ to choose · ⏎ to insert · esc to close` |
| Removed (tooltip) | `Feature removed` |
| Mention tooltip | `<title> · <project>` |
| Placeholder | unchanged: `Message the Overwatch · ⏎ to send` |

## 7. Screen reader

- Textarea: `role="combobox"`, `aria-autocomplete="list"`, `aria-expanded`, `aria-controls="ow-mention-list"` (set only while the popup is open: the id does not exist otherwise), `aria-activedescendant` = the highlighted option's id. `aria-describedby` keeps `ow-input-hint`.
- Popup list: `role="listbox"`, `id="ow-mention-list"`, `aria-label="Features"`. Groups: `role="group"` with `aria-labelledby` the header (when headers are shown, else none). Options: `role="option"`, `aria-selected`; the name is `<slug>, <title>, <project>, <state label>` (`No run` when none): `add-mention-in-overwatch, Add Mention In Overwatch, Orchestrator, Executing`. Decorative bits (dot, emphasis spans) `aria-hidden`.
- The no-match / no-features rows are not options (`role="presentation"`-ish text), the count is announced instead.
- Live region (the existing `.sr`, `aria-live="polite"`, which lives in `ChatColumn.svelte`: `ChatInput` reports the message up, e.g. an `onannounce` prop), updated on open and on each query change, debounced to the settle of typing: `3 features` · `1 feature` · `No feature matches` · `No features yet`. On open, once, `Use up and down arrows to choose, enter to insert, escape to close` (the hint's text, since the footer is dropped at 300 px).
- Mention link name `Open feature @slug` (`aria-label`); removed token read as `@slug (feature removed)`.

## 8. Frames for ui (names kept)

All in the Drive `Orchestrator.figcraft`, P4.4 · 04.3 chat column page, Tokyo Night dark (light variant: ui's call, same tokens), exported under `design/`. Column frame 360 × 800 unless said; thread above with a few real entries (one user entry with the accent wash), input block at the foot with the placeholder.

| File | Shows |
|---|---|
| `mention-popup-open.png` | state 1, 360 px: field holds `@`; popup with **Current** (3 rows, a mix of Executing / Spec gate / PR open) and **Other features** (2 rows, one `No run`), row 1 highlighted; footer hint. |
| `mention-popup-filtered.png` | state 2, 360 px: field `@ment`; 3 rows, matched chars emphasised, row 2 highlighted (to show highlight ≠ first), a single group so **no headers**, footer hint. |
| `mention-popup-empty.png` | states 3 and 4 side by side, 360 px each: `No feature matches “xyz”` (field `@xyz`) and `No features yet` (field `@`). |
| `mention-popup-narrow.png` | 300 px column: state 5, the same slug in two projects (headers shown: one Current, one Other), title hidden, project + state kept, **no footer**, a long slug truncated on a third row. |
| `mention-sent.png` | states 6 and 7, two entries: a sent user entry containing `@add-mention-in-overwatch` at rest, and again on hover (with tooltip) and focus; an older entry with the removed token (muted, tooltip `Feature removed`). |
| `mention-plan-tag.png` | state 8, 360 px: `Planning · <project> ✕` tag above the field, popup above the tag, tag visible. |

Hand the exports to front under `design/` (PNG); a link-only hand-off is not enough.

## Changes to the spec (lead decides)

None changes the data model; all are behaviour details the spec left open or implies differently:
1. **No match / no features, ⏎ and ⇥**: close the popup, keep the text, send nothing (spec §3.2 "⏎ never sends while the popup is open" is kept; this says what ⏎ does when there is nothing to pick). Popup stays closed until the token changes.
2. **The popup opens with no features** and shows `No features yet` (spec silent).
3. **Group headers only when both groups have rows** (brief: a header for every non-empty group).
4. **Popup is the column's full width, anchored to the input block** (planning tag stays visible), not the textarea's width.
5. **Footer hint dropped under 340 px wide**; its text goes to the live region once on open (spec §3.2 doesn't mention the hint, so it needs a line).
6. **Title column dropped at 300 px**, project and state kept.
7. **Open for lead, data model: two features with the same slug in two projects.** Both can be picked (§3 state 5), but the sent text holds the same `@<slug>` token for both, so `meta.mentions[].slug` cannot tell the UI which id a token links to (spec §3.3), the Overwatch cannot tell which one the text means, and spec §3.2's "token still in the text" check cannot tell which pick it belongs to. Options: insert `@<project>/<slug>` when the slug is not unique among non-archived features (the query charset already allows `/`), or refuse a second same-slug pick. Until settled, front links the token to the first matching mention and keeps both ids in the context.
8. **Open for lead, token boundary.** The query charset includes `.`, so `@add-mention-in-overwatch.` or `…?` at the end of a sentence: the rendering and "whole token" rules (spec §3.2, §3.3) must trim trailing `.` and punctuation from a token and match the longest `meta.mentions` slug (so `@add` does not link inside `@add-mention-in-overwatch`).

## ui (boards, contrast)

Boards: Drive `Orchestrator.figcraft`, page 04.3 Chat column, row at y 15700 (`mention-popup-open`, `-filtered`, `-empty`, `mention-plan-tag`, `mention-popup-narrow`, `mention-sent`), exported here as PNG (2×). Contrast is WCAG 2 for 13 px text (4.5:1), dark and light tokens.

- **Matched characters: the fallback, on every row and in both themes.** Accent on `--color-surface-selected` is 4.28:1 (dark) and 2.25:1 (light); in light, accent on `--color-surface-raised` (a row at rest) is 2.41:1 too. So matched chars are `--font-weight-semibold` + `--color-text-primary` + underline (`text-decoration-thickness: 1px; text-underline-offset: 2px`) on all rows, highlighted or not: one rule, and the emphasis looks the same as the highlight moves. Text-primary is 6.68:1 on the selected row (light 5.81:1).
- **Mention hover: the background stays at the 22% mix.** Text-primary on the 38% mix is 3.65:1 (light 4.26:1); on 22% it is 4.73:1 (light 4.87:1). Hover is the underline only, in `currentColor` (text-primary), plus the tooltip.
- **Removed mention: `--color-text-secondary`, not muted.** Muted on the accent-subtle wash is 3.73:1; secondary is 5.11:1 (light 4.74:1). Still no pill, no underline, not a link.
- **Highlighted row: the muted parts go `--color-text-secondary`.** Muted on `--color-surface-selected` is 3.73:1 (light 4.30:1), so on the highlighted row the title, the project caption and `No run` take text-secondary (5.11:1, light 4.74:1); at rest they stay muted (4.65:1 on raised, light 4.62:1). This replaces §2's "the title stays `--color-text-muted`".
- **List height includes its padding.** The app is `box-sizing: border-box`, so the list's `max-height` adds its `--space-xs` top and bottom padding: `min(8 × 32 px + headers × 20 px + 2 × --space-xs, 50vh)`, else 8 rows already scroll.
- These bullets override §1, §2 and §4 where they differ (matched chars, hover background, removed colour, highlighted-row muted parts). Everything else as written above.
- `mention-plan-tag.png` shows two **Current** rows (PR open, Planning), so no group headers, per §1.
