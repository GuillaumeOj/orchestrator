# Add External Files To Project: UX brief

## Where

Project settings pane (`ui/src/panes/ProjectSettingsPane.svelte`), a new
section **Files** in the left nav, after General and before Task backend
(General · Files · Task backend · Allocation · Notion · MCP overrides). Same
layout as the other sections (FigCraft page `04.7 Settings`, the Project
settings Linear and Allocation boards). Draw it at 1512, check it at 1280.

## Content (top to bottom)

1. **Heading** "Files", one line under it: "Files and folders outside the
   repository that every agent of {project} can use: a brand guide, a design
   file, shared notes." Second muted line: "Agents read them in place. They
   aren't copied, versioned or committed."
2. **Drop zone** (always visible, above the list): a dashed-outline block,
   icon + "Drop files or folders here, or **click to browse**" and a muted
   line "Several at once is fine". It is a button: a click (or `⏎`/space when
   focused, or `⌘O` anywhere in the section) opens the native macOS open
   panel to browse the computer (files and folders, multi-select); the
   chosen paths are added as if dropped. Hover and focus: outline in the
   accent color, pointer cursor. Dragging files from Finder over the section
   highlights the zone (solid accent outline, tinted fill) and its text
   becomes "Drop to add to {project}". Compact when the list isn't empty
   (one line high), taller in the empty state.
   Under it, a path field "or paste paths, one per line (~ works)" with an
   **Add** button (`⏎`; a multi-line paste adds every line).
3. **List**, one row per entry, in the order added:
   - icon file / folder, the path shortened with `~` (mono), full path in a
     tooltip; a muted badge **missing** (warning tone) when it no longer
     exists;
   - **note** inline field, placeholder "What is it for? Agents see this",
     saved on blur / `⏎`;
   - **access** segmented control: **Read & write** · **Read only**;
   - row actions (icons, on hover and focus): Reveal in Finder, Open in
     editor (files only), Remove.
4. **Footer note** (muted), shown when the list isn't empty: "Running agents
   get changes when they next start · {n} live now". Hidden when 0 live.

## States to draw

1. **Empty**: heading, the tall drop zone carrying the empty-state text: "No
   files yet. Add a brand guide, a design file or notes that live outside the
   repo." above "Drop files or folders here, or click to browse", then the
   path field.
2. **Filled**: 4 rows: a folder `~/Brand` (note "Brand guide: logo, colors,
   tone", Read & write), a file `~/design/app.figcraft` (no note, Read &
   write), a file `~/specs/partner-api.yaml` (note "Partner API", Read only),
   a missing file `~/old/pricing.pdf`.
3. **Dragging over**: the filled state with the zone highlighted and "Drop
   to add to {project}".
4. **Zone hover/focus**: the compact zone hovered (accent outline, "click to
   browse" emphasized), to show it is clickable. The native panel itself is
   the system's: not drawn.
5. **Batch with refusals**: after adding 4 paths, 2 added (rows highlighted
   briefly) and an inline message block under the drop zone listing the
   refused ones with their reason, e.g. "`~/.ssh`: holds credentials, can't
   be shared with agents" and "`~/dev/shop-api/src`: already part of the
   project (in its repository)". Dismiss with ×. A toast "Added 2 files to
   shop-api" is fine.
6. **Remove confirm**: inline on the row, not a modal: "Remove from the
   project? The file stays on disk." Remove / Cancel.
7. **Load error**: the section shows the error text and a Retry button.
8. **Read & write hint**: the first time an entry is set (or added) Read &
   write, a muted line under the list: "Agents may change these files. Two
   agents writing the same file: the last write wins, nothing undoes it." It
   stays while any entry is Read & write.

## Keys

- `⌘O` (or `⏎`/space on the focused drop zone) opens the native browse
  panel · `⏎` in the path field adds · `esc` clears the field or
  cancels a remove confirm.
- `↑/↓` move between rows; `r` toggles access of the focused row; `⌫`
  starts remove confirm; `⏎` on a row focuses its note.
- Show them with the existing `Kbd` component like the other sections.

## Copy rules

Access labels: "Read & write", "Read only". Kind: "file", "folder". Error
texts come from the core as is (see api-contract.md); the UI doesn't rewrite
them, it shortens the home path to `~`.
