# Add External Files To Project: design notes (Project settings › Files)

For ui (frames) and front (build). Source: `ux-brief.md` (fixed where noted in
"Changes to the brief"), `spec.md`, `api-contract.md`. Patterns follow
`ProjectSettingsPane.svelte`, `AllocationSettings.svelte` and
`docs/design/p3-design-notes.md` §5.13 / §6.

Sample project everywhere: **shop-api**. Frames at 1512, checked at 1280, in
the FigCraft page `04.7 Settings`, next to the other Project settings boards.
Exports in `design/` of this feature.

## Changes to the brief

1. **Nav order**: the pane's `SECTIONS` become General · **Files** · Task
   backend · Allocation · Notion · MCP overrides (as the brief says); the
   nav label is "Files".
2. **Keys**: the pane already uses `j`/`k` (and arrows) for rows, `⏎` to act
   on a row, `esc` to cancel, `r` for Retry/Sync, and writes `Kbd` as
   `⌘+K`, `⌘+⏎`. The brief's `r` (toggle access) clashes with Retry `r`, so
   access toggles on **`a`**. `j`/`k` added next to `↑/↓`. `⌘O` is written
   `⌘+O`.
3. **No Save/Cancel footer.** Other sections save with `⌘+⏎`; Files edits
   are commands applied at once (add, access, note on blur, remove), so the
   section has none. A one-line muted hint states it (see Footer hint).
4. **Read & write hint** is shown *while any entry is Read & write*, not "the
   first time": no hidden "seen" state to store, and it stays true.
5. **Refusals block** uses the pane's warn box (`▲`, amber border, same as
   "State isn't mapped"), not red: part of a batch succeeded. Whole batch
   refused uses the same block with the toast omitted.
6. **Load error** uses the Allocation pattern: red box `✕`, Retry `r`.
7. **Remove confirm**: Remove `⏎` / Cancel `esc` with `Kbd`, like the D8 and
   Allocation confirms.
8. **Toasts** follow §6.2 (past tense + object); "files" fixed to "items"
   since a batch can mix files and folders.
9. **Footer live note**: the whole line is hidden when 0 agents are live;
   otherwise shown. (The brief said "hidden when 0 live" without saying
   which part.)

## Layout

Section body, same column as Task backend / Allocation: `h2`, muted `lead`
lines, then blocks separated by `--space-md`. Rows use `--size-row`. Mono
(`--text-mono`) for paths. Icons: Material Design names, ui picks the
FiraCode NF codepoints and adds them to the README glyph table: file,
folder, folder-open (reveal), open-in-new / pencil (editor), delete / close,
alert (missing), tray-arrow-down (drop zone).

Order top to bottom: heading · lead · drop zone · path field · refusals
block (when present) · list · access hint · live note · key hint.

## Copy (final strings)

`{name}` = project name; `{n}` numbers; `~` replaces the home path.

| Where | String |
|---|---|
| Nav | Files |
| Heading | Files |
| Lead line 1 | Files and folders outside the repository that every agent of {name} can use: a brand guide, a design file, shared notes. |
| Lead line 2 (muted) | Agents read them in place. They aren't copied, versioned or committed. |
| Drop zone, idle | Drop files or folders here, or **click to browse** |
| Drop zone, idle sub (muted, tall only) | Several at once is fine. |
| Drop zone, hover/focus | same text, accent outline, "click to browse" underlined |
| Drop zone, drag-over | Drop to add to {name} |
| Drop zone, empty (above the idle line, muted) | No files yet. Add a brand guide, a design file or notes that live outside the repo. |
| Drop zone aria-label | Add files or folders. Opens the file browser. |
| Path field label (above, muted) | or paste paths, one per line (~ works) |
| Path field placeholder | ~/Brand |
| Path field button | Add `⏎` |
| Row note placeholder | What is it for? Agents see this |
| Row note too long | Note: 500 characters at most (core text, shown under the field) |
| Access labels | Read & write · Read only |
| Access segment tooltip | Agents can change this file · Agents can only read this file (folder: "…this folder's files") |
| Kind (tooltip/aria) | file · folder |
| Row path tooltip | the full canonical path |
| Missing badge | missing (▲, warning tone) |
| Missing row tooltip | Not found on disk. Agents are told it is missing. |
| Row actions (tooltips) | Reveal in Finder · Open in editor · Remove `⌫` |
| Remove confirm | Remove from the project? The file stays on disk. (folder: "The folder stays on disk.") · Remove `⏎` · Cancel `esc` |
| Refusals block title | Not added ({n}) |
| Refusals lines | the core text as is, path shortened to `~` (see samples) |
| Refusals dismiss | × (aria-label "Dismiss") |
| Toast, added | Added {n} item(s) to {name} ("Added 1 item" when 1) |
| Toast, nothing added | none (the block says it) |
| Toast, access | {path} is now Read only / Read & write |
| Toast, removed | Removed {path} from {name} |
| Read & write hint | Agents may change these files. Two agents writing the same file: the last write wins, nothing undoes it. |
| Footer live note | Running agents get changes when they next start · {n} live now |
| Footer key hint | Changes apply as you make them. `j` `k` rows · `⏎` note · `a` access · `⌫` remove · `⌘+O` browse |
| Load error | ✕ Couldn't load {name}'s files: {error} · Retry `r` |
| Loading | skeleton rows (3), aria-busy, no text |
| Action error (toast, error kind) | the core text as is |

Rules as in §6: sentence case, no exclamation marks, errors from the core
unmodified except `~`.

## The 8 states

Common to 2 to 8: the list has 4 rows in the order below, live agents = 3.

**Rows (sample data)**

| # | kind | path | note | access | other |
|---|---|---|---|---|---|
| 1 | folder | `~/Brand` | Brand guide: logo, colors, tone | Read & write | |
| 2 | file | `~/design/app.figcraft` | (empty, placeholder shown) | Read & write | |
| 3 | file | `~/specs/partner-api.yaml` | Partner API | Read only | |
| 4 | file | `~/old/pricing.pdf` | (empty) | Read & write | **missing** badge, actions Reveal and Open in editor disabled, Remove enabled |

1. **Empty.** Heading, leads, tall drop zone carrying the empty text and the
   idle line and "Several at once is fine.", the path field below. No list,
   no hints, no live note.
2. **Filled.** The four rows above under a compact one-line drop zone and the
   path field. Under the list: Read & write hint, then the live note
   ("Running agents get changes when they next start · 3 live now"), then the
   key hint. Row 1 is selected (selected-surface fill).
3. **Dragging over.** Filled state; zone solid accent outline, tinted fill,
   text "Drop to add to shop-api". Nothing else changes.
4. **Zone hover/focus.** Filled state, compact zone with accent outline,
   pointer cursor, "click to browse" emphasized; focus ring if focus. The
   native panel is not drawn.
5. **Batch with refusals.** The user dropped 4 paths: `~/Docs/guidelines.pdf`
   and `~/Docs/logos` added (two new rows at the bottom of the list, tinted
   for ~2 s, then plain), so the list has 6 rows. Under the path field, a warn
   block "Not added (2)":
   - `~/.ssh`: holds credentials, can't be shared with agents
   - `~/dev/shop-api/src`: already part of the project (in its repository)

   with × at its right. Toast "Added 2 items to shop-api".
6. **Remove confirm.** Filled state; row 2 is replaced inline by the confirm:
   "Remove from the project? The file stays on disk." with Remove `⏎` (danger
   tone) and Cancel `esc`. Other rows stay interactive.
7. **Load error.** Heading and leads kept; below them the red box "✕
   Couldn't load shop-api's files: could not read the settings store" and
   Retry `r`. No zone, no list.
8. **Read & write hint.** Filled state cropped on the list bottom: the hint
   line and live note shown. Same sample as state 2; additionally a variant
   with all rows Read only where the hint is absent (small inset, optional).

## Keyboard map

The section is inside the pane's `region`; keys act when focus is not in an
input/textarea/select and there are no modifiers, except `⌘+O`.

| Key | Where | Does |
|---|---|---|
| `⌘+O` | anywhere in the section, also in the fields | opens the native browse panel |
| `⏎` / `space` | focused drop zone | same |
| `⏎` | path field | adds the pasted lines (empty field: nothing) |
| `esc` | path field | clears it; elsewhere cancels a remove confirm, then closes the refusals block |
| `j` / `↓`, `k` / `↑` | list | next / previous row |
| `⏎` | row | focuses its note field |
| `⏎` / `esc` | note field | saves and blurs / reverts and blurs |
| `a` | row | toggles Read & write ⇄ Read only |
| `⌫` | row | starts the remove confirm |
| `⏎` / `esc` | remove confirm | Remove / Cancel |
| `r` | load error | Retry |
| `tab` / `shift+tab` | all | follow the focus order |

`Kbd` renders keys exactly as written above (`⌘+O`, `⌫`).

## Focus order

1. Section nav (existing).
2. Drop zone (a `button`).
3. Path field, then Add.
4. Refusals block × (when shown).
5. Rows, top to bottom; inside a row: note field, access segments (radio
   group, arrows move inside), Reveal, Open in editor, Remove. Row action
   icons are visible on row hover and whenever a control in the row has
   focus; always in the tab order.
6. Remove confirm replaces the row's controls: focus goes to Remove, `esc`
   returns focus to the row.
7. Load error: Retry.

After adding, focus stays on the zone (keyboard path) or the path field; the
new rows get `aria-live="polite"` text "Added {n} items".

## What is NOT drawn

- The native macOS open panel (files and folders, multi-select): the system's
  own, opened by `pick_paths`. Frames show only the zone before and after it.
- Finder, the Finder drag ghost and cursor during a drag: state 3 shows only
  the app side.
- Reveal in Finder / Open in editor results (external apps).
- The agent prompt "Project files" section and the permission rules
  (see spec.md).
- Other sections' content (only the nav entry changes).

## Open for ui

- Row height with the inline note: single line, note field takes the free
  width between path and access; at 1280 the path truncates in the middle
  (`~/design/…/app.figcraft`) before the note shrinks below 160 px.
- Dark tokens only, as the rest of the app; accent outline uses the existing
  accent token, warning tone the amber pacing token (as `warnbox`).
