# Fix Remove Worktree · API contract

> Between `back` (core, Tauri commands) and `front` (`ProjectPane.svelte`). Unchanged command names and arguments: nothing breaks for other callers.

## `worktree_remove` (Tauri) → `Runtime::remove_worktree`

Arguments are unchanged: `{ project: string, path: string, deleteBranch: bool, discard: bool }`.

Returns `RemoveOutcome` (unchanged shape):

```json
{ "worktree_removed": true, "branch_deleted": true, "log": ["unlocked", "removed worktree …", "deleted branch orch/f/back-1"] }
```

New guarantees:

- **After `Ok`**, the next `project_detail` for that project doesn't list `path`, whatever the case: folder present, leaf or feature folder gone, unknown to git with a row, or no row.
- **After `Ok`**, `project:<project id>` is touched (the UI's `onTopics` fires).
- **Idempotent:** if git doesn't know `path` and its row is already `Removed` (or there is none), the call returns `Ok` with `worktree_removed: false, branch_deleted: false, log: []`, changes nothing and touches no topic.
- **`missing` worktree** (row live, git doesn't know it): the row's `branch` is deleted when `deleteBranch` (`-d`, or `-D` with `discard`). A failure is logged as `kept branch …` and doesn't fail the call.
- Errors are unchanged (strings): `… is not in …: the app only removes its own worktrees`, `agent <id> uses this worktree: archive or kill it first`, `…: not merged into <base> … (needs confirmation)`, `…: N uncommitted file(s) (needs confirmation)`.

## `project_detail` → `worktrees[]`

Each entry is `WorktreeInfo` as today, plus one field:

| Field | Type | Meaning |
|---|---|---|
| `in_use_by` | `string[]` | Ids of agents that are neither archived nor killed and whose worktree is this one (canonical path). Empty when free. While non-empty, `worktree_remove` is refused. |

`in_use_by` is always present, `[]` when free, so the UI can rely on it. `delete_plan`'s `shared_with` keeps its meaning.
