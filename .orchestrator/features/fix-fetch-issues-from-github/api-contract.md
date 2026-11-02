# Fix Fetch Issues From GitHub · API contract

Tauri commands between `ui/src/panes/BoardPane.svelte` and `crates/orch-app` (`commands.rs` → `teams.rs` → `agent-runtime`). Additions only; existing fields keep their meaning.

## `task_update` (changed)

```js
invoke('task_update', { id, status, node, note, title, body })
```

| arg | type | meaning |
|---|---|---|
| `title` | `string \| null` | new title (trimmed); `""` → error `the task needs a title`; `null` = unchanged |
| `body` | `string \| null` | new body (trailing whitespace trimmed); `null` = unchanged, `""` clears it |

Returns the task (as today). A remote failure keeps the local change (the task goes pending, synced later), as for status today. Title/body unchanged → no remote write.

## `board_open_sync` (new)

```js
invoke('board_open_sync', { project }) // → { synced: bool, report?: SyncSummary }
```

The Board calls it on mount (and when the project changes). The core syncs the project's remote backend if its last sync is older than 60 s and records the project as **watched** (the 5-min background sync now covers it for the rest of the session). `synced: false` = local backend or synced recently (nothing done). A sync error is thrown like `board_sync` (the Board shows it in its `backend disconnected` line). `SyncSummary` = what `board_sync` returns today (`{pushed, pulled, imported, updated, conflicts, errors, unmapped_states}`).

After it returns `synced: true`, the Board reloads `board_view` (the `now` topics may also do it; a reload is cheap).

## `board_view` (changed)

Each task gains:

| field | type | meaning |
|---|---|---|
| `remote_url` | `string \| null` | the issue's web URL (`https://github.com/<owner>/<repo>/issues/<n>`) for a GitHub task with an issue; the Linear issue URL when known; else `null` |

## `task_create` (unchanged)

Already takes `form.body`; the Board now sends it.

## Opening a URL

The Board opens `remote_url` with the app's existing external-link helper (the same one the Feature pane uses for PR links); no new command.
