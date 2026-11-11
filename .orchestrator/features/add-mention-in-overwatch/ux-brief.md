# Add Mention In Overwatch · UX brief

> For ux (final interaction in `design/design-notes.md`) and ui (boards, exports under `design/`). Behaviour rules: [spec.md](spec.md) §3.2–3.3. Tokens: the app's existing ones (`ui/src/styles/tokens.css`); no new colours.

## 1. Where

The Overwatch column (FigCraft P4.4 · 04.3 chat column page), its input at the foot ("Message the Overwatch · ⏎ to send", main's copy) and the thread above. Draw the column at its default 360 px width, and one board at the 300 px minimum to check truncation.

## 2. The popup

- Opens **above** the input field, anchored to the field (not the caret), the field's full width, over the thread (a raised surface, the app's menu / palette style: the ⌘P list is the reference). It never pushes the thread or the input.
- Up to 8 rows visible, then the list scrolls; the highlighted row stays in view.
- Groups: **Current** (features with a live run) then **Other features**, each with a small caption header; a group with no rows is not shown.
- Footer hint line (caption, muted): `↑↓ to choose · ⏎ to insert · esc to close`. ux decides whether it earns its place at 300 px.

## 3. A row

Single line, 32 px high:
- `@slug` in mono, the matched characters emphasised (bold or accent: ux / ui choose, as ⌘P does if it does).
- Title, muted, truncated with an ellipsis.
- Right: project name (caption) and the run state as the existing state label / badge ("Executing", "Spec gate", "PR open", … or "No run").
- Highlighted row: the existing list selection treatment. Hover = highlight.

## 4. States to settle (ux) and draw (ui)

1. **Open, empty query** (`@` just typed): current first, then others.
2. **Filtered** (`@ment`): a few rows, matches emphasised.
3. **No match**: one non-selectable row "No feature matches “xyz”" (copy to confirm); ⏎ then inserts nothing and does not send (ux: or closes the popup and keeps the text?).
4. **No features at all**: "No features yet" (or the popup does not open: ux decides).
5. **Same slug in two projects**: both rows, the project name telling them apart.
6. **The sent message**: a user entry whose text contains `@add-mention-in-overwatch` rendered as a mention link inside the accent-washed bubble (a subtle pill or an underlined accent link: must read as clickable on the wash and stay text-sized, no line-height jump). Hover / focus state.
7. **Mention of a removed feature** in an older entry: plain muted token, tooltip "Feature removed".
8. **With the planning tag** shown above the field (P4.5): the popup sits above both, or covers the tag (ux decides).

## 5. Keys and copy to confirm

- ↑ / ↓, ⌃P / ⌃N move; ⏎ and ⇥ insert; Esc closes the popup only; a click inserts.
- Inserted text `@<slug> ` (trailing space).
- Screen reader: options read "<slug>, <title>, <project>, <state>"; the count announced: "3 features" / "No feature matches".
- Anything in spec §3.2 that reads wrong in use (trigger chars, ordering, the 8 rows): change it in design-notes and tell lead; the notes win over the spec on copy and visuals, lead settles behaviour changes.

## 6. Boards (ui) and exports

Export PNGs to `.orchestrator/features/add-mention-in-overwatch/design/`:
- `mention-popup-open.png` — state 1, 360 px.
- `mention-popup-filtered.png` — state 2, with a row highlighted.
- `mention-popup-empty.png` — states 3 and 4 side by side.
- `mention-popup-narrow.png` — state 2 or 5 at 300 px (truncation).
- `mention-sent.png` — states 6 and 7 (rest / hover).
- `mention-plan-tag.png` — state 8.

Frames live in the Drive `Orchestrator.figcraft` (never commit `docs/design/Orchestrator.figcraft`).
