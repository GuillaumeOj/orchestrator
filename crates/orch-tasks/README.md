# orch-tasks

Task backends (ADR-004, FR-501–503). One task model (`orch_store::Task`, columns of
`orch_team::TaskStatus`) is mirrored in the store for every backend, so the board and the team
runtime read one table. Agents never call a backend: the orchestrator MCP tools go through
`TaskService`.

| Backend | Where tasks live | Writes | Reads |
|---|---|---|---|
| **Local** | the store | the store | the store |
| **GitHub Issues** | issues with the `orch` label | write-through with `gh api` (the user's `gh` login; an update reads the issue first to keep its other labels) | `sync`: `gh issue list --label orch --state all` |
| **Linear** | issues of one team (and project) with the `orch` label | write-through with GraphQL (`linear.rs`, through a `Graphql` transport the core gives: the Keychain key over curl) | `sync`: `issues(filter: team, label orch)`, paged |

`TaskService::new(store, project, gh)` picks the project's backend (`projects.task_backend`,
`{"type":"github","repo":"owner/name"}`; no repo = the checkout's). API: `create`, `update`,
`set_status`, `note` (a comment on GitHub), `sync(now) -> SyncReport { imported, updated, pushed,
conflicts, errors }`. A failed push keeps the local change (`Outcome.remote_error`) and marks the
task pending; `sync` pushes it again.

Both remotes implement `Remote` (list, create, update, get, comment); the sync is the same for
both. A task whose `backend` is not the project's (left local when the backend changed, D8) is
never pushed.

`Backends::service(store, project, gh, linear)` does the same but keeps one remote backend per project for
the life of the process (the runtime holds one `Backends`), so the repository is looked up and the
labels are created once, not on every write. A change of the project's backend setting or checkout
makes a new one.

## GitHub mapping

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
| pending | changed | **adopt the remote** and report it (`conflicts`): the caller tells the user |
| never created remotely | — | create the issue (`pushed`) |
| — | new issue with `orch` | import (`imported`) |

Tests: `cargo test -p orch-tasks` (an in-memory GitHub behind the `Gh` trait: write-through,
labels, dependencies, comments, import, offline edits, conflicts).
