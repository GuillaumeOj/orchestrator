# Allow Long Text In Overwatch · UX brief

> For ux (final interaction in `design/design-notes.md`) and ui (boards, exports under `design/`). Behaviour rules: [spec.md](spec.md) §3.2–3.3. Tokens: the app's existing ones (`ui/src/styles/tokens.css`); no new colours. Reference: the Claude Code CLI's `[Pasted text #1 +42 lines]`.

## 1. Where

The Overwatch column (FigCraft P4.4 · 04.3 chat column page, next to the mention boards), its input at the foot ("Message the Overwatch · ⏎ to send") and the thread above. Draw at the default 360 px width, and one board at the 300 px minimum to check wrapping.

## 2. In the input

- The input is a plain `textarea`: the placeholder is **text** (`[Pasted text #1 · 5 000 chars]`), it cannot be styled inside the field. ux confirms the exact wording (spec: `[Pasted text #<n> · <chars> chars]`; adding lines, e.g. `· 120 lines`, is ux's call, keeping it short enough to sit on one line at 300 px).
- What the user sees right after a paste: the placeholder at the caret, the field not growing by the paste. ux decides whether a brief caption under the field confirms it ("Pasted 5 000 characters as #1 · Backspace removes it"), in the hints row, and for how long.
- Backspace just after / Delete just before a placeholder removes it whole. ux: is that enough, or does a removed paste need an undo hint? (⌘Z restores it.)
- Errors (caption, error colour, in the place of today's over-length warning): paste over 200 000 chars; 11th paste. Copy to write.
- The over-length warning (typed text > 2 000) gets new copy pointing to paste (spec §3.2).
- The planning tag and the mention popup are unchanged; draw one board with both a placeholder and a mention in the draft.

## 3. In the thread

- The user's sent entry (accent-washed bubble): each placeholder renders as a **paste chip**, inline in the text, text-sized (no line-height jump), clearly clickable on the wash and distinct from a mention link: e.g. a small document glyph + `Pasted text #1 · 5 000 chars · 120 lines`.
- Click / ⏎ / Space toggles an **expansion** under the bubble (or inside it: ux decides) showing the pasted text: mono, pre-wrapped, max ~16 lines then scroll inside, a Copy button, and a way to collapse. Several pastes: each chip opens its own; one open at a time or many (ux decides).
- Loading (the text is fetched on first open): a short loading state.
- Missing file: chip muted, expansion "Pasted text no longer available".
- An unsent (refused) entry with chips, with its Retry.

## 4. Accessibility

- Chip: a button, `aria-expanded`, label "Pasted text 1, 5 000 characters, 120 lines".
- After a paste becomes a placeholder, announce it in the existing live region ("Pasted 5 000 characters as pasted text 1").

## 5. Boards to draw (ui)

1. Input, empty → after a long paste: placeholder in the field, confirmation caption (if ux keeps it).
2. Input with typed text + two placeholders + a mention, planning tag shown.
3. Input errors: too long a paste; 11th paste; typed text over 2 000 (new copy).
4. Thread: sent entry with one chip, collapsed.
5. Thread: same entry, expanded (long text scrolling, Copy), hover / focus states of the chip.
6. Thread: two chips in one entry, one expanded; missing-file state; loading state.
7. Thread: unsent entry with a chip and Retry.
8. 300 px width: boards 2 and 5.

Anything in spec §3.2–3.3 that reads wrong in use (thresholds, wording, Backspace rule): change it in design-notes and tell lead; the notes win on copy and visuals, lead settles behaviour changes.
