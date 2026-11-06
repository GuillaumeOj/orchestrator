# Fix Linear Pull · Feature spec

> Feature `fix-linear-pull` · Integration branch `orch/fix-linear-pull/integration` · Backend (`orch-tasks`) + one line of copy (Project settings) · no design pass, no API change

## 1. Problem

Since `fix-linear-sync`, a Linear sync on Fusily finishes without the "Query too complex" error, but the Board stays (almost) empty: the team's Linear issues are not pulled.

## 2. Cause (HEAD 5882f8b)

**The Linear listing only asks for issues with the `orch` label.** `LinearIssues::list` (`crates/orch-tasks/src/linear.rs`) sends:

```
issues(first: 25, filter: { team: { id: { eq: <team> } }, labels: { name: { eq: "orch" } }, project?: … })
```

Only issues the app created itself carry `orch`. Checked live on the Fusily workspace (team `FUS`, ~1 500 issues): only **FUS-1541** and **FUS-1542**, both created by the app, have `orch`. Every other issue (hundreds open, in Backlog, Todo, In Progress) is never listed, so `TaskService::sync` never imports it.

This is the bug `fix-fetch-issues-from-github` fixed for GitHub. That spec left Linear out on purpose ("Linear keeps its full listing and its behaviour (its `orch` filter is out of scope)"). This feature brings Linear in line with GitHub.

Two things go with it:

1. **`to_issue` hardcodes `managed: true`**: true while the listing asks for `orch`, false once it reads foreign issues. The sync uses `managed` to skip closed issues that aren't the app's.
2. **A Linear update sends every field.** `LinearIssues::update` always sends `title`, `description`, `stateId`, `labelIds` and rewrites the `blocks` relations, whatever `IssueWrite.fields` says. Today that only touches the app's own issues. Once foreign issues are on the Board, moving a card would overwrite a title or description someone changed in Linear since the last sync. GitHub already sends only the changed fields.

The sync triggers are fine. Board open (`open_sync`), the `r` key, and the 5-min background sync (`sync_task_backends`) all work the same for every backend.

## 3. Fix

### 3.1 Read every open issue, then only the changed ones (back · `linear.rs`, `lib.rs`)

Same model as GitHub (`GithubIssues::list`, `Coverage`, the store's sync cursor):

- **First sync (no cursor):** every issue of the team (and the project, when one is set) whose state type is not `completed` or `canceled`, **or** that has the `orch` label. One `IssueFilter` with `or: [{ state: { type: { nin: ["completed", "canceled"] } } }, { labels: { name: { eq: "orch" } } }]`. Coverage `OpenAndManaged`: the shared sync already reads alone a tracked issue that isn't listed (pending, or not Done).
- **Later syncs (cursor):** every issue of the team (and project) with `updatedAt > cursor − 60 s`, all states. Coverage `ChangedSince`.
- **Cursor** = the newest `updatedAt` seen, never older than the `since` it was asked with (like GitHub). `Remote for LinearIssues` implements `list_issues(since)` with it and `cursor_scope()` = `linear:<team key>` or `linear:<team key>/<project id>`. The store already drops the cursor when the project's backend setting changes, and the scope check covers a team or project change.
- **Paging:** keep `issues(first: 25)` (the `OrchIssues` cost guard from `fix-linear-sync` stays ≤ 5 000). Raise the page cap from 50 pages (1 250 issues) to 400 pages (10 000 issues), so a big team's first sync isn't silently cut short.
- **`managed`** = the issue has the `orch` label (no longer hardcoded).
- An imported issue takes its column from its state (the project's mapping, else the state type), as today. A closed issue the app doesn't track and that has no `orch` label is not imported (the shared sync already skips it). **Reading never writes to Linear:** no label is added on import.

### 3.2 Writes send only what changed (back · `linear.rs`)

`LinearIssues::update` follows `w.fields`, like `GithubIssues::update`:

- `fields.state` (status or node change): `stateId` and `labelIds`. `labelIds` = `orch` + `orch:node:<id>` + the issue's other labels, read first, as today. This is how a foreign issue becomes managed: its first status change in the app adds `orch`.
- `fields.content` (title, body or dependencies change): `title`, `description`, and the `blocks` relations are matched to `blocked_by`.
- `fields.state` only: no `title` or `description` is sent and the relations are not touched.
- `create` and a pending re-push still send everything (`Fields::ALL`).
- Remote wins on conflict (FR-503), as today.

### 3.3 Copy (front · `ProjectSettingsPane.svelte`)

The Linear note in Project settings › Task backend says *"Issues labelled orch in {team} sync both ways."* Make it say what the app now does:

> Open issues of {team} (and of the project, when one is set) show on the Board. Moving or editing a card writes it back to Linear and adds the label orch. When both sides changed, Linear wins and you get a note in today's digest. Assignee node: label orch:node:&lt;id&gt;. Dependencies: Linear "blocks" relations.

Update any UI test that pins the old sentence.

### 3.4 Fake and tests (back · `linear.rs` fake, `tests.rs`)

- `fake::FakeLinear` answers `OrchIssues` **by its filter**: team, optional project, the `or` (open state type, or label `orch`), `updatedAt.gt`, and paging (`first` / `after`, `hasNextPage`). Fake issues get a project and an `updatedAt` that orders correctly beyond 59 ticks (today it's `00:00:{:02}`).
- Tests (in `orch-tasks`):
  1. A team with open issues **without** `orch` (and a completed one without `orch`): the first sync imports the open ones in their state's column. The completed one is not imported. Nothing is written to Linear (no mutation in `calls`).
  2. More than 1 250 open issues: all are imported (paging past the old cap).
  3. Second sync: only the issues changed since the cursor are asked (`updatedAt` filter in the variables), and an issue edited, moved to Done, or reopened in Linear shows up. The cursor is kept in the store with scope `linear:ENG`.
  4. Moving a foreign card to In Progress sends `stateId` + `labelIds` (with `orch`, keeping its other labels) and **no** `title` / `description`. Editing its title sends `title` + `description` and no `stateId`.
  5. Offline: a pending status change whose issue didn't change (so absent from the incremental list) is pushed at the next sync. With the issue changed, Linear wins (`conflicts`).
  6. With a project set: only that project's issues are listed. The scope is `linear:ENG/<project>`.
  7. The `fix-linear-sync` cost guard still passes for the new `OrchIssues` text (the `or` filter is a variable, so it costs nothing).
- The existing `orch-app` `core.rs` Linear tests keep passing.

### 3.5 Docs (back · `crates/orch-tasks/README.md`)

Linear row of the backend table, the Linear mapping section, the "what a write sends" paragraph ("Linear sends every field" goes away), the coverage paragraph, the cursor paragraph (scope `linear:…`), and the sync table row "new open issue (GitHub: any labels; Linear: with `orch`)".

## 4. Out of scope

- A per-project label or state filter (which Linear issues show on the Board, beyond team and project).
- Importing the closed history (completed or canceled issues without `orch`).
- Archived or deleted Linear issues: a tracked one is left as it is (same as GitHub "gone").
- Linear comments, assignees, priority, estimate, cycles, sub-issues.
- Any change to the Board itself. It already shows Linear cards, "Open on Linear", the sync line and its errors.

## 5. Acceptance

1. On a Linear-backed project, the open issues of the team (or project) that have no `orch` label show on the Board after opening it, with no `r`, each in the column of its state.
2. Moving such a card adds `orch` in Linear, keeps its other labels, and leaves its title and description untouched.
3. A Linear-side edit, close or reopen shows on the Board after the next sync. A completed issue the app never tracked and that has no `orch` is not imported.
4. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `make lint` and `bun run test` in `ui/` pass.
5. Manual check by the user on Fusily (the sandbox has no Linear API key and no access to api.linear.app): the Board of the Fusily project lists the open FUS issues, and the sync line reads "N pushed · N pulled · N conflicts".

## 6. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Linear reads every open issue, incrementally; partial writes | back | §3.1, §3.2, §3.4, §3.5 in `crates/orch-tasks` (`linear.rs`, `lib.rs` `Remote for LinearIssues`, `tests.rs`, README). `model: opus`. |
| T-2 Project settings Linear note | front | §3.3, `ui/src/panes/ProjectSettingsPane.svelte` (+ UI test). No dependency. Reviews T-1 when back asks. |
