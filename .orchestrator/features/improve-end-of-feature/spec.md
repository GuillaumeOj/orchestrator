# Improve the end of a feature · Feature spec

> Feature `improve-end-of-feature` · Integration branch `orch/improve-end-of-feature/integration` · Touches: SPEC §6.2 (team run states), FR-106 (PRs and CI via `gh`), FR-401 (Lead protocol), ADR-009 (only the app pushes)
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (the PR line and the new run state)

## 1. Goal

A feature is **done when its PR is merged on GitHub**, not when the PR is opened. Between the two, the team stays on the feature so it can fix what the PR needs: a CI failure caused by the feature, or a branch that is behind `main` or conflicts with it. The app tracks the PR (state, CI, position against its base) and shows it next to the PR in the Feature pane, in a short form: **`PR #<n> [CI] [position]`**.

## 2. What the code does today (HEAD 68ea76d)

| Item | Today |
|---|---|
| End of a run | The PR gate is approved → `open_pr` pushes and runs `gh pr create` → `pr_opened` stores `run.pr_url` and moves the run **`pr_gate → done`** (`agent-runtime/src/team.rs:1617`). `done` is final (`RunState::finished`). |
| After `done` | The team tick, task sync and Notion polling skip the run (`runs(.., active_only)`); every team tool answers "the team run is Done: nothing more to do" (`team.rs:1131`). Nobody can fix a red CI or a conflict without a new feature. |
| PR data | Only `run.pr_url` (a string). No PR number, state, CI or merge state is stored. `orch_git::gh::prs` lists open PRs with a CI rollup (`ChecksSummary`) for the Project pane only. |
| Pushing again | `request_pr` refuses a second pending gate and always runs `gh pr create`: no way to push new commits to an open PR. |
| Base branch | No tool brings `origin/<base>` into the integration branch; specialists have `sync_integration` (integration → their branch) only. |
| UI | Feature pane, card "PR and artifacts": `PR <full url>` as plain text, not clickable (`FeaturePane.svelte:291`). |

## 3. Scope

### 3.1 New run state `pr_open`
- Lifecycle becomes `… integrating → pr_gate → pr_open → done`. The PR opening moves the run to **`pr_open`** (label **"PR open"**), not `done`.
- Moves: `pr_gate → pr_open` (PR opened), `pr_open → done` (PR merged), `pr_open → integrating` (PR closed without merge), plus pause / defer / fail from `pr_open` as from any unfinished state. `pr_open` is not finished: the team tick, task sync, Notion poll and team tools work as in `integrating`.
- `tasks_allowed()` includes `pr_open`: the lead can create tasks to fix the PR, and merge them as usual.
- `done` keeps its meaning (final), now reached **only** by a merged PR. `ended_at_ms` is set then.
- Runs already `done` stay `done` (no backfill).

### 3.2 PR record on the run
- `TeamRun` gains `pr_number: Option<u64>` and `pr: Option<PrStatus>` (stored as JSON in `team_runs.pr_status`; one migration adds `pr_number INTEGER` and `pr_status TEXT`). `pr_url` stays.
- `PrStatus` = `{ state: open|merged|closed, head_sha, base, ci: {overall, passing, failing, pending, skipped, failing_checks: [{name, url}]}, position: {status: up_to_date|behind|conflicts|unknown, behind, ahead}, unpushed, checked_at_ms, error? }`.
  - `ci` from `statusCheckRollup` (the existing `summarize_rollup`, extended with the names and links of failing checks; at most 10).
  - `position.status`: `conflicts` when GitHub says `mergeable: CONFLICTING` (or `mergeStateStatus: DIRTY`); else `behind` when `behind > 0`; else `up_to_date`; `unknown` while GitHub computes mergeability or the base was never fetched. `behind` / `ahead` = `git rev-list --left-right --count origin/<base>...<head_sha>` in the integration worktree.
  - `unpushed` = commits on the local integration branch not in `head_sha` (work merged by the lead, waiting for a push).
- `pr_opened` parses the number from the URL `gh pr create` prints, stores `pr_number`, and does a first status check at once.

### 3.3 PR watcher (core)
- A `pr_tick` on the background fetch thread (network calls stay off the delivery loop), **every 60 s**, for each run in `pr_open` (or paused/deferred with `resume_status = pr_open`):
  1. `gh pr view <n> --json number,url,state,headRefOid,baseRefName,mergeable,mergeStateStatus,statusCheckRollup` in the integration worktree (new `orch_git::gh::pr_view`, 30 s deadline).
  2. `git fetch origin <base>` at most every 5 min per run (same offline / battery rules as FR-103: paused when offline or on battery below 20 %), then the behind / ahead count.
  3. Store the new `PrStatus`. A `gh` failure keeps the last status and sets `error` (the UI shows it as stale); it does not change the run.
- `orch pr_refresh` (UI button, `orchd`) runs the same check for one run now, skipping the throttle but not the offline check.
- **Transitions** (each one writes a timeline event and, when it calls for work, one ORCH turn to the lead):
  - `ci.overall` becomes `failing` → lead: `ORCH: CI fails on PR #<n>: <check names and links>. Find whether the feature caused it; if so, fix it with a task (or yourself), merge, and call mcp__orch__request_pr to push the fix. If the failure is not the feature's (flaky, infra), say so with mcp__orch__ask_user.`
  - `ci.overall` becomes `passing` after a failure → timeline only.
  - `position.status` becomes `behind` or `conflicts` → lead: `ORCH: PR #<n> is <behind <base> by N commits | in conflict with <base>>. Call mcp__orch__sync_base, resolve any conflicts, run the tests, commit, then mcp__orch__request_pr to push.` Being behind without conflicts is told once per new base head, not every minute.
  - `state` becomes `merged` → run `pr_open → done` ("pull request merged"), lead told `ORCH: PR #<n> was merged. The feature is done; end your turn.` An Inbox info item "Feature <slug> merged" for the user.
  - `state` becomes `closed` (not merged) → run `pr_open → integrating` ("pull request closed"), lead told, Inbox info item. The lead may ask for a new PR (`request_pr` opens a new one and replaces `pr_url` / `pr_number`).
- Same transition, same message: the watcher compares with the stored status, so a restart does not repeat a message for a status already stored.

### 3.4 Bring the base in: lead tool `sync_base`
- New orchestrator MCP tool for the **lead only** (team tools list, Lead protocol): fetches `origin/<base>` and **merges** it into the integration worktree (`merge_into_worktree(.., abort_on_conflict = false)`, the same helper as `sync_integration`). Conflicts stay in the worktree for the lead to fix and conclude with `mcp__orch__commit`; specialists then `sync_integration` as today.
- Allowed while the run is `integrating`, `pr_gate` or `pr_open`.
- **Merge, not rebase**: the PR branch only moves forward, so updates are fast-forward pushes and never need a force push (ADR-009: no history rewrite on a shared remote branch; the repo already merges PRs with merge commits). GitHub's "behind" and "conflicts" both clear after the merge.

### 3.5 Pushing updates to the open PR
- `request_pr` while the run is `pr_open` asks for an **update** instead of a new PR: Inbox gate `action: "push_pr_update"`, title `<lead>: push <unpushed> commit(s) to PR #<n> (<branch>)`, payload `{branch, pr_number, url, commits: [{sha, subject}], worktree, run}`. `title` / `body` arguments are optional then and ignored.
- On approval the core pushes the branch (fast-forward only: a non-fast-forward push fails and is reported, never forced), audits it (`pr_update`), writes a `pr.pushed` timeline event, re-checks the PR at once, and tells the lead `ORCH: the user approved; pushed <n> commit(s) to PR #<n>. CI restarts; you will be told the result.`
- Refused when nothing is unpushed ("nothing to push") or an update gate is already pending.
- Still one gate per push: the user approves every push (ADR-009).

### 3.6 Lead protocol
- Step 7 of the Lead prompt (`orch-team/src/compile.rs`) becomes: *"When every task is done and the integration branch passes its tests, call mcp__orch__request_pr. Once the PR is open the feature is not done: you are told when CI fails or the PR falls behind or conflicts with its base. Fix it (tasks as usual, mcp__orch__sync_base for the base), then mcp__orch__request_pr again to push. The feature is done when the PR is merged."*
- `sync_base` is in the lead's tool list.

### 3.7 UI (front, see [ux-brief.md](ux-brief.md))
- **Feature pane, card "PR and artifacts"**: the full URL is replaced by the short line `PR #<n>` + CI badge + position badge (+ `↑<n> to push` when `unpushed > 0`, + `merged` / `closed` when not open). `PR #<n>` opens the PR in the browser (`openUrl`); the full URL is its tooltip. The CI badge's tooltip lists failing checks (each opens its link). A refresh button calls `pr_refresh`; "checked 2 min ago" is shown, and the line is marked stale when `error` is set.
- **Run state `pr_open`**: label "PR open" and a color in `runStateLabel` / `runStateColor` (sidebar, palette, Feature pane header, timeline). `runFinished` is unchanged (`done`, `failed`): the message box stays while the PR is open.
- **Timeline** texts for `pr.opened`, `pr.status`, `pr.pushed`, `pr.merged`, `pr.closed`.

## 4. Out of scope
- Solo sessions' PRs (no run): unchanged.
- Rebase and force push (see §3.4).
- Merging the PR from the app, auto-merge, review requests or review comments.
- Re-opening runs already marked `done` before this change.
- Webhooks: polling only.

## 5. Non-functional
- **Security (ADR-009)**: agents still never push; every push is a user gate; pushes are fast-forward only; `sync_base` only merges into the integration worktree.
- **Cost**: one `gh pr view` per open-PR run per minute, one `git fetch` of the base per 5 min, none while offline or on low battery.
- **Data (NFR-8)**: one migration (`team_runs.pr_number`, `team_runs.pr_status`); old rows read with `None`.
- **Tests**: `make ci` green. Unit tests: `RunState` moves (`pr_gate → pr_open → done`, `pr_open → integrating`, tasks allowed in `pr_open`); `PrStatus` parsing (merged, closed, CONFLICTING, BEHIND, UNKNOWN, failing names); the watcher's transitions with a fake `gh` (one message per transition, none repeated after a reload, merged → done); `request_pr` in `pr_open` → update gate, refused when nothing unpushed; `sync_base` merge and conflict on a fixture repo. UI: vitest for the PR line formatter and the new state label.

## 6. Acceptance (demo at the PR gate)
1. Approving a PR gate opens the PR and the run shows **PR open**, with `PR #<n>` + CI pending + up to date.
2. A failing check turns the CI badge red, the lead is told once with the check names; after the fix is merged and the update gate approved, the commits are pushed to the same PR and CI runs again.
3. A commit landing on `main` shows `↓1 behind`; the lead is told, calls `sync_base`, and after the push the badge shows up to date. A conflicting change on `main` shows **conflicts**.
4. Merging the PR on GitHub moves the run to **Done** within a minute; the lead is told; the team tools then refuse work.
5. Closing the PR without merging moves the run back to **Integrating**; `request_pr` opens a new PR.
6. Offline: no `gh` call, the line shows the last status as stale.
