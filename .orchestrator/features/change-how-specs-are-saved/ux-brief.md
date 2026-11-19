# UX brief — Feature docs setting

Read `spec.md` §3 first. Design in the shared Figcraft file (Google Drive
`Orchestrator.figcraft`), next to the existing Project settings frames; reuse their
components (nav, section header, radio rows, text field, inline error, footer Save).

## Where

Project settings window, new nav section **Feature docs**, placed after *Files*
(General, Files, Feature docs, Tracker, …).

## Content

Intro, one line: "Where team runs save each feature's spec and designs."

**Docs** row — radio group:
- *In the repository* → a path field, monospace, default `.orchestrator/features/{slug}`,
  with a resolved example under it: `→ .orchestrator/features/checkout/`.
- *Not saved in the repository* → helper: "Kept on this Mac only, never committed.
  Deleted with the feature."

**Designs** row — radio group:
- *With the docs* (default) → example `→ <docs>/design/`.
- *In the repository* → path field + example.
- *Not saved in the repository* → same helper.

Footnote: "Applies to features you start from now on. Running features keep their folders."

## States to draw

1. Default (both rows on their defaults).
2. Docs in a custom folder, designs in another repo folder (two path fields visible).
3. Docs not saved, designs *With the docs* (the designs example reads "Not saved either").
4. Validation error on a path field ("Must contain {slug}", "Must be a path inside the
   repository") with Save disabled; dirty footer as in other sections.
5. Feature page docs block, both variants: "In the repository · `docs/specs/checkout/`"
   and "Not saved in the repository" + **Reveal in Finder** button.

## Rules

- `{slug}` is shown as a token, not explained at length: a short hint "{slug} is the
  feature's short name".
- No modal; inline only. Keyboard: radios with arrows, ⌘⏎ saves.
- Hand off to `ui` for final frames; export each frame to
  `.orchestrator/features/change-how-specs-are-saved/design/<name>.png` and commit.

## Final design (T-463)

Frames in `Orchestrator.figcraft`: page **04.7 Settings**, below the Allocation/Sandbox frames at
y=31002 (`FD · 1…4`, 1512 wide, built from the Tracker · Local frame); **04.4 Feature &
board** at y=17900 (`FD · 5`). Exports: `design/feature-docs-1-default.png`,
`-2-custom-folders.png`, `-3-not-saved.png`, `-4-validation-error.png`,
`feature-page-docs-location.png`.

Final copy (changes from the brief marked *):
- Nav item: **Feature docs** (after Files). Title: Feature docs. Rows: **Docs**, **Designs**.
- Radios: "In the repository" · "Not saved in the repository" · (designs) "With the docs".
- Docs example: `→ .orchestrator/features/checkout/`. Designs *With the docs* example is
  **resolved** *: `→ .orchestrator/features/checkout/design/` (not the `<docs>/design/` token);
  when docs are not saved: `→ Not saved either`.
- Hint under the docs path field only: "{slug} is the feature's short name".
- Errors (red, under the field, field border red): "Must contain {slug}" ·
  "Must be a path inside the repository" (absolute, `..`, `.git`, empty parts) · "Too long (200
  characters at most)" *. While any error shows: Save disabled and "Fix the folders above to
  save." * next to Revert.
- Clean state (state 1): no "Unsaved changes", Save disabled (muted). Dirty: blue dot +
  "Unsaved changes", Save enabled with ⌘⏎.
- Selected radio: filled dot, label semibold; the path field shows only under a selected
  "In the repository" (focused field: blue border). Radios are a single tab stop, arrows move.
- Feature page: line under the docs tabs, above the document: "In the repository · `<path>`"
  (path monospace) or "Not saved in the repository" + **Reveal in Finder** (secondary button).

## Final frames (T-464, ui)

Dark and Light for every state. Figcraft **04.7 Settings**: `FD · 1…4 · 1512` at y=31002 (dark)
and `… · Light` at y=31846. **04.4 Feature & board**: `FD · 5` (y=17900) and `FD · 5c` (y=18330),
dark at x=0, Light at x=1660.

- Exports: `design/feature-docs-{1-default,2-custom-folders,3-not-saved,4-validation-error}[-light].png`,
  `design/feature-page-docs-location[-light].png`, `design/feature-page-docs-location-designs[-light].png`.
- Dirty marker: blue dot + "Unsaved changes" centred on the title line, 16 px after it.
- Feature page, designs outside the docs folder (lead decision): a second line under the docs
  line, "Designs · In the repository · `<path>`" or "Designs · Not saved in the repository" +
  **Reveal in Finder**. Designs *With the docs* (in the repository: under the docs folder; not
  saved: the feature's app folder, where `design/` sits next to `docs/`): docs line only (5a, 5b).
- Path after "In the repository · ": one space after the dot, path in the primary text colour.
- Light palette = the Tokyo Night Day values the other Light settings frames use (window
  `#D0D5E3`, content `#E1E2E7`, text `#283E72`/`#495178`, accent `#2E7DE9`, error `#CE0A43`).
