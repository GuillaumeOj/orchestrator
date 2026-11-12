# API contract · fix-remove-worktrees-outside-the-app

Tauri commands, JSON as the UI sees it. Unchanged fields are not repeated.

## `project_detail({ id, withPrs })` → `worktrees[]`

Each entry gains two fields:

```jsonc
{
  "path": "/Users/u/dev/repo/.claude/worktrees/keen-brattain-df09cf",
  "branch": "claude/keen-brattain-df09cf",
  "verdict": "prunable",        // unchanged set: ok | missing | unknown | prunable | main | external
  "locked": true,
  "lock_reason": "claude agent", // NEW: string | null (git's `locked <reason>`; null when not locked or no reason)
  "outside": true,               // NEW: canonical path is not under <repo>.worktrees/
  "safe_to_remove": true,        // outside rule: not locked, dirty 0, nothing in progress, on a branch or merged HEAD
  "in_use_by": [],
  // … path, head, row, base, dirty, unmerged_commits, merged, in_progress as today
}
```

- `main`: `outside` is false (it's never removable anyway).
- `external` ⇒ `outside: true`. `prunable` and `missing` may be either.

## `worktree_remove({ project, path, deleteBranch, discard })` → `RemoveOutcome`

Same arguments. Now accepted for outside worktrees.

```jsonc
{
  "worktree_removed": true,
  "branch_deleted": false,
  "kept_branch": "claude/keen-brattain-df09cf", // NEW: string | null, set when the branch was kept because unmerged (outside worktrees)
  "log": ["removed worktree …", "kept branch claude/keen-brattain-df09cf: not merged into main"]
}
```

Errors (string, shown as a toast):

- `refusing to remove the main checkout`
- `agent <id> uses this worktree: archive or kill it first`
- `<path> is not a worktree of <project name>` (NEW: neither git's worktree of this repo nor a live row)
- `<path> is locked (<reason>): needs confirmation` (NEW: outside, locked, `discard: false`)
- `<path>: N uncommitted file(s) (needs confirmation)`, `… in progress (needs confirmation)`, `<tip>: not merged into <base> (… needs confirmation)` (unchanged; for outside worktrees the last one only for a detached HEAD)

Removed: `… the app only removes its own worktrees`.

A second removal of something already gone stays an OK no-op (`worktree_removed: false`).

## `worktrees_prune({ project })` → NEW

Prunes every listed worktree of the project whose folder is gone (verdict `prunable` or `missing`), not in use by a live agent. No discard: unmerged branches are kept.

```jsonc
{
  "removed": ["/…/.claude/worktrees/a", "/…/repo.worktrees/feat/back-1"], // canonical paths, as listed
  "kept_branches": ["claude/a"],                                          // unmerged, not deleted
  "deleted_branches": ["orch/feat/back-1"],
  "skipped": [{ "path": "/…/x", "reason": "agent feat-back-1 uses this worktree" }]
}
```

- Errors only for the project itself (unknown project, git not runnable). A per-worktree failure goes to `skipped` with its reason.
- Touches `project:<id>` once; audit `worktrees_pruned`.
- Registered in `orch-app/src/lib.rs` next to `worktree_remove`.
