# Fix Fetch Issues From GitHub · Feature spec

> Feature `fix-fetch-issues-from-github` · Integration branch `orch/fix-fetch-issues-from-github/integration` · Backend (`orch-tasks`, runtime, app commands) + Frontend (Board) · no design pass

## 1. Problem

A project whose task backend is **GitHub Issues** shows an empty (or near-empty) Board even though the repository has open issues. The user wants the Board to be a real view of the repo's issues: **read** them, **change their status**, **create** issues and **edit** them.

## 2. Cause (HEAD 28493ec)

1. **Only `orch`-labelled issues are ever read.** `GithubIssues::list` (`crates/orch-tasks/src/github.rs`) runs `gh issue list --label orch --state all --limit 500`. Issues created on GitHub (by people, templates, other tools) never carry `orch`, so `TaskService::sync` never imports them. This is the bug the user sees.
2. **The read is capped and full every time.** `--limit 500` over `--state all` silently drops issues past 500 (closed ones count) and re-reads every issue on each 5-min sync.
3. **The Board never syncs by itself.** The background sync (`Runtime::sync_task_backends`, every `TASK_SYNC_EVERY_MS`) only covers projects with an **active team run**; otherwise only the Board's `r` syncs. Opening the Board of a GitHub project shows whatever the store last had (nothing, the first time).
4. **A status change rewrites the whole issue.** `GithubIssues::update` PATCHes title **and body** on every write, so moving a card overwrites an edit made on GitHub since the last sync (and normalises the body's line endings).
5. **No way to edit a task's title or body** from the app: `task_update` only takes `status`, `node`, `note`; the Board's new-task form has no body field.

## 3. Scope

### 3.1 Read every issue (back)

- A GitHub sync reads **every open issue** of the repo, with or without `orch`, plus every issue the app already tracks (open or closed), plus closed `orch` issues (today's behaviour). Pull requests are not issues: skipped.
- Read through the REST issues endpoint, **paged** (`gh api --paginate`, `per_page=100`), no 500 cap.
- **Incremental after the first sync:** the backend keeps a per-project cursor (the newest `updated_at` seen, persisted in the store so it survives a restart). With a cursor, the sync asks `state=all&since=<cursor − 1 min>`: that returns new issues, edits, closes and reopens. Without one (first sync, or the backend/repo changed), it reads all open issues + closed `orch` issues.
  - A closed issue the app does not track and that has no `orch` label is **not** imported (history stays on GitHub).
- Because an incremental list no longer returns unchanged issues, the shared `TaskService::sync` must still **push again a pending task whose issue is not in the list** (its remote did not change → same as "pending, remote unchanged"). Linear keeps its full listing and its behaviour (its `orch` filter is out of scope).
- An imported issue with no `orch:<status>` label is **Backlog** when open, **Done** when closed (as `parse_issue` does now). Reading never writes to GitHub (no label is added on import).

### 3.2 Writes touch only what changed (back)

- **Status / node change** (Board move, assign, MCP `update_task`): sends only `labels` (managed `orch`, `orch:<status>`, `orch:node:<id>` + the issue's other labels, re-read first as today) and `state` (closed = Done). Title and body are not sent. This is how a foreign issue becomes managed: its first status change adds `orch` + its status label.
- **Title / body edit:** sends `title` and `body` (with the `Blocked by:` line re-rendered) — and nothing else changes.
- **Create:** unchanged (title, body, labels; Done → closed right after).
- A pending re-push (after an offline write) may send every field, as today.
- Remote wins on conflict (FR-503) as today.

### 3.3 The Board syncs when it opens (back + front)

- Opening the Board of a remote-backed project syncs it when the last sync is older than 60 s (no double sync on quick re-opens); `r` still syncs at once. The Board shows the sync's progress (`Syncing…`) and its error line as today.
- The 5-min background sync covers every project with a GitHub backend whose Board was opened in this app session, in addition to projects with an active run. (No sync of projects the user never looks at.)

### 3.4 Edit a task (back + front)

- `task_update` takes optional `title` and `body`; a change is saved and pushed (3.2). Empty title refused.
- Board: `e` on a card (and **Edit…** in the card menu) opens an inline form with the title and a body textarea; ⏎ in the title / ⌘⏎ in the body saves, Esc cancels.
- Board new-task form (`n`) gets a body textarea.
- Each GitHub card offers **Open on GitHub** (card menu, key `g`): opens the issue URL in the browser.

### 3.5 Docs

- `crates/orch-tasks/README.md`: GitHub row (reads every open issue, incremental `since`, writes only the changed fields), mapping notes (a foreign issue becomes managed on its first status change), sync table (pending + not listed = push again).

## 4. Out of scope

Linear changes; importing closed issue history; issue comments shown in the app; GitHub assignees, milestones, Projects (v2) fields; label filters per project; deleting or transferring issues.

## 5. Acceptance

1. A GitHub project whose repo has open issues **without** `orch` shows them on the Board (Backlog, or their `orch:<status>` column) after opening the Board, with no `r`.
2. A repo with > 500 issues: every open one is imported (fake gh test with paging).
3. Moving such a card to In progress adds `orch` + `orch:in_progress` on GitHub, keeps its other labels, and does not send title/body (test: the PATCH has no `title`/`body` field). Moving to Done closes it; out of Done reopens it.
4. An issue closed / edited / reopened on GitHub shows on the Board after the next sync (incremental `since`); a closed untracked non-`orch` issue is not imported.
5. Editing title/body on the Board updates the issue (test: PATCH has `title` + `body`, no `labels`/`state` change).
6. Creating a task on the Board with a body opens an issue with that body.
7. Offline: a status change is kept pending; at the next sync with the issue unchanged (so absent from the incremental list) it is pushed; with the issue changed, the remote wins and the card shows `↓ changed in GitHub`.
8. Cursor survives a restart (store test) and is reset when the backend or repo changes.
9. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `make lint` and `bun run test` in `ui/` pass.

## 6. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 GitHub sync reads every issue, incrementally; partial writes | back | `orch-tasks` (`github.rs`, `lib.rs` sync, tests + fake gh paging/since), cursor in the store, README |
| T-2 Edit + sync-on-open + background coverage | back | `task_update` title/body (runtime `user_update_task`, server, app command), Board-open sync command / throttle, `sync_task_backends` covers opened projects, `remote_url` in `board_view` |
| T-3 Board: edit, body, open on GitHub, sync on open | front | `BoardPane.svelte` (+ `lib/teams.js` helpers, tests), depends on T-2 (contract in `api-contract.md`) |
