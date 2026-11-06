# Add External Files To Project: API contract

Tauri commands (`crates/orch-app/src/commands.rs`, registered in `lib.rs`),
called with `invoke` from the UI. Paths in and out are strings. All errors
are plain text (`Err(String)`), shown as is.

## Types

```jsonc
// ProjectFile, as stored (ProjectSettings.external_files) and returned
{
  "path": "/Users/me/Brand",          // canonical, absolute
  "kind": "folder",                    // "file" | "folder"
  "access": "write",                   // "write" | "read"
  "note": "Brand guide: logo, colors", // omitted when none
  "added_at_ms": 1762430000000,
  "exists": true                       // computed on each read, not stored
}
```

## Commands

### `project_files(project: string) -> { files: ProjectFile[], live_agents: number }`

The list in stored order, `exists` checked now; `live_agents` = live agents
of the project (for the footer note).

### `project_files_add(project: string, paths: string[], access?: "write"|"read") -> { files: ProjectFile[], added: string[], refused: {path: string, error: string}[] }`

`paths` as typed, picked or dropped (`~/…` expanded by the core). Default
access `write`. Each path is validated alone (spec.md › Validation);
`added` holds the canonical paths added, `refused` the input path with its
message. `files` is the whole list after the change. Duplicates inside one
call (same canonical path) count once: added once, never in `refused`. An unknown project is an `Err`.

### `project_file_update(project: string, path: string, access?: "write"|"read", note?: string) -> { files: ProjectFile[] }`

`path` is the stored canonical path. Only the fields given change: `note`
absent = unchanged, `note: ""` (or only spaces) clears it (trimmed, max 500 chars, else
`note: 500 characters at most`). Unknown path: `Err("{path}: not in the
project files")`.

### `project_file_remove(project: string, path: string) -> { files: ProjectFile[] }`

Removes the entry, never the file. Unknown path: same `Err` as above.

### `pick_paths() -> string[]`

The native open panel (`tauri-plugin-dialog`), files and folders, multiple
selection; `[]` when cancelled. Called when the user clicks the drop zone
(or `⌘O`); the UI passes the result to `project_files_add`.

### Existing commands

- `project_settings(project)` also returns `settings.external_files` (as
  stored, no `exists`); the UI uses `project_files` for the section.
- `project_settings_set(project, form)` keeps the stored `external_files`
  whatever `form.settings` carries.
- Reveal in Finder: new `reveal_path(path: string) -> null` (back, T-257):
  `open -R`, only for a path listed in some project's files, else
  `Err("{path}: not in the project files")`.
- Open in editor: the existing FR-1101 open-in-editor path (front finds the
  command the panes and diffs use). If it only accepts worktree paths, front
  tells back.

## Drag and drop

Front listens to the Tauri webview drag-drop event
(`getCurrentWebview().onDragDropEvent`), which carries real paths, while the
Project settings › Files section is shown, and sends `paths` to
`project_files_add`. The event is window-wide: front hit-tests its position
against the section's box (the visible Files section of the pane under the
pointer; split panes may show two), and ignores drops elsewhere. Nothing on
the backend beyond `project_files_add`.

## Agent side (no command; for reference)

`CompileCtx::project_files: Vec<ProjectFile>` filled by `Runtime::spawn`
from the agent's project; `compile` adds the permission rules,
`allowWrite`, `--add-dir` and the prompt section described in spec.md › At
spawn.
