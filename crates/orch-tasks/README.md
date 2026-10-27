# orch-tasks

Task backends (ADR-004, FR-501–503). One task model (`orch_store::Task`, columns of
`orch_team::TaskStatus`) is mirrored in the store for every backend, so the board and the team
runtime read one table. Agents never call a backend: the orchestrator MCP tools go through
`TaskService`.

| Backend | Where tasks live | Writes | Reads |
|---|---|---|---|
| **Local** | the store | the store | the store |
| **GitHub Issues** | issues with the `orch` label | write-through with `gh api` (the user's `gh` login; an update reads the issue first to keep its other labels) | `sync`: `gh issue list --label orch --state all` |

`TaskService::new(store, project, gh)` picks the project's backend (`projects.task_backend`,
`{"type":"github","repo":"owner/name"}`; no repo = the checkout's). API: `create`, `update`,
`set_status`, `note` (a comment on GitHub), `sync(now) -> SyncReport { imported, updated, pushed,
conflicts, errors }`. A failed push keeps the local change (`Outcome.remote_error`) and marks the
task pending; `sync` pushes it again.

## GitHub mapping

- `orch` marks managed issues; `orch:<status>` the column (`orch:backlog`, `orch:ready`,
  `orch:in_progress`, `orch:review`; a **closed** issue is Done); `orch:node:<id>` the assignee
  node. Other labels are kept on every update. The status labels are created once per process.
- Dependencies are a last body line `Blocked by: #12, #13` (issue numbers), parsed back into
  local task ids.
- A task's handle is `#<number>` on GitHub, `T-<id>` locally.

## Sync and conflicts (FR-503: the remote wins)

`remote_updated_at` holds the issue's `updatedAt` after every successful write or read. At sync:

| Local | Remote since last seen | Result |
|---|---|---|
| unchanged | unchanged | nothing |
| unchanged | changed | adopt the remote (`updated`) |
| pending (push failed) | unchanged | push again (`pushed`) |
| pending | changed | **adopt the remote** and report it (`conflicts`): the caller tells the user |
| never created remotely | — | create the issue (`pushed`) |
| — | new issue with `orch` | import (`imported`) |

Tests: `cargo test -p orch-tasks` (an in-memory GitHub behind the `Gh` trait: write-through,
labels, dependencies, comments, import, offline edits, conflicts).
