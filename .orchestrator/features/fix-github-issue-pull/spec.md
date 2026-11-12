# Fix GitHub Issue Pull · Feature spec

> Feature `fix-github-issue-pull` · Integration branch `orch/fix-github-issue-pull/integration` · Backend (`orch-tasks` intake readers, `agent-runtime` intake status) + one line of Frontend (Ready line) · no design pass

## 1. Problem

On BIMCraft (`GuillaumeOj/BIMCraft`, ~320 issues and PRs), the issues of **Epic #67 "3D viewport"** never show up in the orchestrator. Issues of later epics (#79 Georeferencing, #84 Materials) do. Nothing tells the user that anything was left out.

What we know about #67 (read from the public API): it is open, its labels are `enhancement`, `area:geometry`, `area:ui` **and `orch`**, and it is one of the repo's older issues.

## 2. Cause (HEAD f841c01)

The app reads GitHub issues in two places:

- the **Board**, through the task backend sync (`orch-tasks/src/github.rs`, `lib.rs`);
- the Mission's **Ready to start** lane, through the intake readers (`orch-tasks/src/intake.rs`).

The Board sync already pages everything (`gh api --paginate`, fixed in `fix-fetch-issues-from-github`), and its incremental cursor is reset on a backend, checkout or project change. **The intake readers do not page.** Each one reads **one batch of at most 100 raw items**, and only then filters, sorts and caps:

1. **Projects v2** (`project_items`): `items(first: 100)` with no `pageInfo`. A board with more than 100 items (closed and Done items count too) only gives its first 100 in board order. The Status filter (`Ready`, `Todo`…) runs **after** the cut, so any matching item past the 100th is gone, whatever its status. The issues the cut keeps depend on the board's order, not on age.
2. **Label / milestone** (`rest_issues`): one REST page (`per_page=100`, oldest first) and no `--paginate`. Matching open issues past the 100th are never read, and `total` (the lane's "+ n more") undercounts.
3. **Linear** (`LINEAR_QUERY`): `issues(first: 100)` and no paging. Priority and sort order are applied to those 100 only.

In all three cases the item is silently missing, and "+ n more in <source>" doesn't count it.

There is a second, deliberate filter: **an issue that carries any `orch` / `orch:*` label is never an intake item** (P4.5 decision D4: it is a Board task). Epic #67 carries `orch`, so the lane hides it **by design** whatever the paging does. It shows on the Board (Backlog, or the column its `orch:<status>` names).

## 3. Scope

### 3.1 Intake readers read the whole source (back)

- **Projects v2**: page `items(first: 100, after: $cursor)` through `pageInfo { hasNextPage endCursor }` until the end. Apply the Status filter, the issue and open checks and the `orch` exclusion to every page. Keep the board order. Then `Listing::capped` (CAP 50 shown, `total` = every match).
- **Label / milestone**: `gh api --paginate` over `repos/{repo}/issues?state=open&…&sort=created&direction=asc&per_page=100`. Parse every page, as the Board's `pages` already does: `gh` prints one JSON array per page, back to back. The read stays a GET (no `-f`), and the order stays oldest first.
- **Linear**: page `issues(first: 100, after: $after)` through `pageInfo`, then sort by priority and sort order over the whole set. The existing rate-limit back-off wraps each page.
- **Bound**: at most **20 pages** (2,000 raw items) per source per refresh. Past that, the refresh keeps what it read: `Listing.partial = true`, `total` = the matches read. The source status JSON gains `partial` (api-contract.md), and the lane says "+ n+ more". No error. A test covers the bound.
- Every read stays read-only (D3); `assert_read_only` covers the paged calls.
- The P4.5 cost line "≤ 1 request page per source per 10 min" becomes "≤ 1 refresh per source per refresh interval, paged". The refresh interval itself doesn't change.

### 3.2 The `orch` exclusion stays (no change)

An `orch`-labelled issue is still a Board task and still left out of the lane (D4). Epic #67 is therefore expected on the **Board**, not in Ready to start. That gets one line in the docs (3.3). If the user wants plain-`orch` issues back in the lane, it is a separate product change: say so at approval.

### 3.3 Docs (back)

- `crates/orch-tasks/README.md` (intake part, or a short "Intake sources" note if there is none): every source is read in full, paged, bounded at 2,000 items. An `orch`-labelled issue is a Board task, never intake.
- `.orchestrator/features/p-4-5-intake/decisions.md`: no edit (old feature). This spec records the cost-line change.

## 4. Out of scope

- The Board / task-backend sync. It already pages, and this spec found no defect in it. If the user confirms the missing issues are missing from the **Board** (not from Ready to start), that is a new diagnosis.
- Changing D4 (the `orch` exclusion).
- Any other UI change.
- ETag / `since` for intake.

## 5. Acceptance

1. Projects v2 fake with 250 items over 3 pages, where the 180th and 240th are open issues in the filtered Status: both are listed, in board order, and `total` counts every match (test).
2. Label filter fake with 130 matching open issues over 2 pages: `total` = 130, the first 50 shown are the oldest, `more()` = 80. The calls are GETs with `--paginate` (test).
3. Linear fake with 150 issues over 2 pages: the highest priority sits on page 2 and comes first (test).
4. A source with more than 20 pages stops at 20, with `partial: true` and the `2000+` count (test).
5. An `orch`-labelled issue is still left out, on every page (test).
6. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` and `make lint` pass.
7. On BIMCraft, with the user's intake source, the Ready to start lane lists the Epic #67 issues that match the filter and carry no `orch` label. #67 itself shows on the Board. (User check at the PR gate.)

## 6. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Page every intake reader | back | `orch-tasks/src/intake.rs` (`project_items`, `rest_issues`, `linear` + their queries), `Listing.partial`, `agent-runtime/src/intake.rs` status `partial`, tests with paged fakes, README note |
| T-2 "+ n+ more" when partial | front | `ui/src/components/mission/ReadyLine.svelte` (+ its test). Depends on T-1 (contract in `api-contract.md`) |
