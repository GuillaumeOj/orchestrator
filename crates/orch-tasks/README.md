# orch-tasks

Task backends (ADR-004, FR-501–503). One task model (`orch_store::Task`, columns of
`orch_team::TaskStatus`) is mirrored in the store for every backend, so the board and the team
runtime read one table. Agents never call a backend: the orchestrator MCP tools go through
`TaskService`.

| Backend | Where tasks live | Writes | Reads |
|---|---|---|---|
| **Local** | the store | the store | the store |
| **GitHub Issues** | every open issue of the repo, the closed ones the app tracks or that have `orch` | write-through with `gh api` (the user's `gh` login), **only the changed fields**: a status/node change sends `labels` + `state` (reading the issue first to keep its other labels), a title/body edit `title` + `body` | `sync`: REST `repos/{repo}/issues`, paged (`gh api --paginate`, 100 per page): first all open + closed `orch` issues, then only `state=all&since=<cursor − 1 min>` |
| **Linear** | issues of one team (and project) with the `orch` label | write-through with GraphQL (`linear.rs`, through a `Graphql` transport the core gives: the Keychain key over curl) | `sync`: `issues(filter: team, label orch)`, paged |

`TaskService::new(store, project, gh)` picks the project's backend (`projects.task_backend`,
`{"type":"github","repo":"owner/name"}`; no repo = the checkout's). API: `create`, `update`,
`set_status`, `note` (a comment on GitHub), `sync(now) -> SyncReport { imported, updated, pushed,
conflicts, errors }`, `github_repo()` (`owner/name`, for issue URLs). A failed push keeps the local change (`Outcome.remote_error`) and marks the
task pending; `sync` pushes it again.

Both remotes implement `Remote` (list, create, update, get, comment); the sync is the same for
both. `list_issues(since)` returns a `Listing { issues, complete, cursor }`: Linear always lists
in full (no cursor); GitHub lists in full without a cursor and incrementally with one.

What a write sends is decided against the stored task: status or node changed → `state`, title,
body or dependencies changed → `content` (`IssueWrite.fields`); a task the remote never got or a
pending one sends everything; nothing GitHub shows changed → no call. Linear sends every field.

**Cursor.** The newest `updated_at` a GitHub sync saw, per project, in the store
(`Store::task_sync_cursor`, kv `task_sync_cursor:<project>`, with its scope `github:owner/name`),
so it survives a restart. It is dropped when the project's backend setting or checkout changes
(`upsert_project`, `set_project_settings`; a no-op update keeps it) and ignored when its scope is
not the repository the backend resolves. A task whose `backend` is not the project's (left local when the backend changed, D8) is
never pushed.

`Backends::service(store, project, gh, linear)` does the same but keeps one remote backend per project for
the life of the process (the runtime holds one `Backends`), so the repository is looked up and the
labels are created once, not on every write. A change of the project's backend setting or checkout
makes a new one.

## GitHub mapping

- Every open issue is on the board: without an `orch:<status>` label it is Backlog (closed =
  Done). Reading never writes; a foreign issue becomes managed (`orch` + its status label) on its
  first status change in the app. Pull requests are skipped.
- `orch` marks managed issues; `orch:<status>` the column (`orch:backlog`, `orch:ready`,
  `orch:in_progress`, `orch:review`; a **closed** issue is Done); `orch:node:<id>` the assignee
  node. Other labels are kept on every update. The status labels are created once per backend.
- Dependencies are a last body line `Blocked by: #12, #13` (issue numbers), parsed back into
  local task ids.
- A task's handle is `#<number>` on GitHub, `T-<id>` locally.

## Linear mapping

- `orch` marks managed issues; the workflow state is the column: by state type (`backlog`/`triage`
  → Backlog, `unstarted` → Ready, `started` → In progress, or Review when its name says "review",
  `completed`/`canceled` → Done), overridden per project by `states: {stateId: column}`. Writing a
  column picks the first state (by position) that maps to it.
- `orch:node:<id>` (a team label, created on first use) is the node; other labels are kept.
- Dependencies are `blocks` issue relations (created and deleted to match `depends_on`).
- An issue's key (`external_id`) is its identifier (`ENG-142`), which Linear takes as an id.
- Rate limits: a 429 or a `RATELIMITED` error stops this backend's calls (Retry-After, else 60 s
  doubling to an hour); `linear::fake::FakeLinear` is Linear in memory for tests.

## Sync and conflicts (FR-503: the remote wins)

`remote_updated_at` holds the issue's `updatedAt` after every successful write or read. At sync:

| Local | Remote since last seen | Result |
|---|---|---|
| unchanged | unchanged | nothing |
| unchanged | changed | adopt the remote (`updated`) |
| pending (push failed) | unchanged | push again (`pushed`) |
| pending | not in an incremental list (so unchanged) | push again (`pushed`) |
| pending | not in a full list (closed without `orch`) | read the issue alone, then as above |
| pending | changed | **adopt the remote** and report it (`conflicts`): the caller tells the user |
| never created remotely | — | create the issue (`pushed`) |
| — | new open issue (GitHub: any labels; Linear: with `orch`) | import (`imported`) |
| — | closed issue, untracked, without `orch` | nothing (history stays on the remote) |

Tests: `cargo test -p orch-tasks` (an in-memory GitHub behind the `Gh` trait, with paging and
`since`: write-through, partial writes, labels, dependencies, comments, import of > 500 issues,
incremental sync, the cursor across a restart, offline edits, conflicts).
