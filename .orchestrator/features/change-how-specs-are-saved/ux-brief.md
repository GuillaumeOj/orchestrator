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
