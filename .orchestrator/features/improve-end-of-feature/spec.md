# Improve the end of a feature · Feature spec

> Feature `improve-end-of-feature` · Integration branch `orch/improve-end-of-feature/integration` · Touches: SPEC §6.2 (team run states), FR-106 (PRs and CI via `gh`), FR-401 (Lead protocol), ADR-009 (only the app pushes)
> Revision 2 (user, spec gate): **rebase** the feature onto its base (merge stays for task branches and for the PR into `main`); CI shown as **`x/y` jobs** with a colored dot and the full job list in the tooltip.
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (the PR line and the new run state)

## 1. Goal

A feature is **done when its PR is merged on GitHub**, not when the PR is opened. Between the two, the team stays on the feature so it can fix what the PR needs: a CI failure caused by the feature, or a branch that is behind `main` or conflicts with it. The app tracks the PR (state, CI, position against its base) and shows it next to the PR in the Feature pane, in a short form: **`PR #<n> [CI] [position]`**, CI being a colored dot and `x/y` jobs finished.

## 2. What the code does today (HEAD 68ea76d)

| Item | Today |
|---|---|
| End of a run | The PR gate is approved → `open_pr` pushes and runs `gh pr create` → `pr_opened` stores `run.pr_url` and moves the run **`pr_gate → done`** (`agent-runtime/src/team.rs:1617`). `done` is final (`RunState::finished`). |
| After `done` | The team tick, task sync and Notion polling skip the run (`runs(.., active_only)`); every team tool answers "the team run is Done: nothing more to do" (`team.rs:1131`). Nobody can fix a red CI or a conflict without a new feature. |
| PR data | Only `run.pr_url` (a string). No PR number, state, CI or merge state is stored. `orch_git::gh::prs` lists open PRs with a CI rollup (`ChecksSummary`) for the Project pane only. |
| Pushing again | `request_pr` refuses a second pending gate and always runs `gh pr create`: no way to push new commits to an open PR. |
| Base branch | No tool brings `origin/<base>` into the integration branch; specialists have `sync_integration` (integration → their branch) only. `mcp__orch__commit` concludes a merge in progress, not a rebase. |
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
- `PrStatus` = `{ state: open|merged|closed, head_sha, base, ci: {overall, done, total, jobs: [{name, workflow?, state, url?}]}, position: {status: up_to_date|behind|conflicts|unknown, behind, ahead, base_sha}, pushed_sha, unpushed, rewritten, checked_at_ms, error? }`.
  - `ci` from `statusCheckRollup` (new `summarize_jobs` next to `summarize_rollup`). One **job** per CheckRun / StatusContext; its `state` is the GitHub value lower-cased: `success`, `neutral`, `skipped`, `failure`, `cancelled`, `timed_out`, `action_required`, `startup_failure`, `stale`, `error` (finished) or `in_progress`, `queued`, `waiting`, `requested`, `pending`, `expected` (not finished); any other value counts as not finished (waiting), never as ok. At most 100 jobs in `jobs`, failures first, then running, waiting, then the rest, each group by name.
  - `total` = number of jobs on the PR head, counted before the 100 cap (GitHub's `totalCount` when the rollup is paged), so `total > jobs.length` means some jobs were left out; `done` = finished jobs, whatever their outcome (ok, failed, cancelled or skipped).
  - `overall` (the dot's color): `error` (red) when any finished job is `failure`, `cancelled`, `timed_out`, `action_required`, `startup_failure`, `stale` or `error`; else `running` (yellow) when any job is `in_progress`; else `waiting` (grey) when any job is not finished (queued, waiting, requested, pending, expected); else `ok` (green) when `total > 0`; `none` when there is no job.
  - `position.status`: `conflicts` when GitHub says `mergeable: CONFLICTING` (or `mergeStateStatus: DIRTY`); else `behind` when `behind > 0`; else `up_to_date`; `unknown` while GitHub computes mergeability or the base was never fetched. `behind` / `ahead` = `git rev-list --left-right --count origin/<base>...<head_sha>` in the integration worktree; `base_sha` = the `origin/<base>` head they were counted against.
  - `pushed_sha` = the PR head the app itself last pushed (set by `open_pr` and every approved update push).
  - `unpushed` = commits on the local integration branch not in `head_sha`; `rewritten` = the local branch does not contain `pushed_sha` (a rebase happened: the next push replaces the PR's history). A `head_sha` that differs from `pushed_sha` means someone else pushed to the PR (e.g. GitHub's "Update branch"): it is never taken as the lease, so a force push over it is refused.
- `pr_opened` parses the number from the URL `gh pr create` prints, stores `pr_number`, and does a first status check at once.

### 3.3 PR watcher (core)
- A `pr_tick` on the background fetch thread (network calls stay off the delivery loop), **every 60 s**, for each run in `pr_open` (or paused/deferred with `resume_status = pr_open`):
  1. `gh pr view <n> --json number,url,state,headRefOid,baseRefName,mergeable,mergeStateStatus,statusCheckRollup` in the integration worktree (new `orch_git::gh::pr_view`, 30 s deadline).
  2. `git fetch origin <base> <head branch>` at most every 5 min per run (the PR head must be local for the counts below) (same offline / battery rules as FR-103: paused when offline or on battery below 20 %), then the behind / ahead count.
  3. Store the new `PrStatus`. A `gh` failure keeps the last status and sets `error` (the UI shows it as stale); it does not change the run.
- `orch pr_refresh` (UI button, `orchd`) runs the same check for one run now, skipping the throttle but not the offline check.
- **Transitions** (each one writes a timeline event and, when it calls for work, one ORCH turn to the lead):
  - `ci.overall` becomes `error` → lead: `ORCH: CI fails on PR #<n>: <failed job names, states and links>. Find whether the feature caused it; if so, fix it with a task (or yourself), merge, and call mcp__orch__request_pr to push the fix. If the failure is not the feature's (flaky, infra), say so with mcp__orch__ask_user.`
  - `ci.overall` becomes `ok` (from any other value) → timeline only. Running / waiting changes are not events (the line shows them).
  - `position.status` becomes `behind` or `conflicts` → lead: `ORCH: PR #<n> is <behind <base> by N commits | in conflict with <base>>. Call mcp__orch__rebase_base, resolve any conflicts, run the tests, then mcp__orch__request_pr to push.` Being behind without conflicts is told once per new base head (`position.base_sha`, stored, so a restart does not repeat it), not every minute.
  - `state` becomes `merged` → run `pr_open → done` ("pull request merged"), lead told `ORCH: PR #<n> was merged. The feature is done; end your turn.` An Inbox info item "Feature <slug> merged" for the user. A pending `push_pr_update` gate is withdrawn.
  - `state` becomes `closed` (not merged) → run `pr_open → integrating` ("pull request closed"), lead told, Inbox info item, a pending `push_pr_update` gate withdrawn. The lead may ask for a new PR (`request_pr` opens a new one and replaces `pr_url` / `pr_number`).
- Same transition, same message: the watcher compares with the stored status, so a restart does not repeat a message for a status already stored.

### 3.4 Bring the base in: lead tool `rebase_base`
- New orchestrator MCP tool for the **lead only**: fetches `origin/<base>` and **rebases** the integration branch onto it in the integration worktree (`git rebase --no-autosquash --rebase-merges origin/<base>`, new `orch_git::commit::rebase_worktree`, run by the core as the merges are). Task merges into the integration branch and the PR into `main` stay merges; only the feature branch is rebased.
- **Preconditions** (else refused with the reason): the run is `integrating`, `pr_gate` or `pr_open`; the worktree is clean; no merge or rebase is in progress; no PR gate or `push_pr_update` gate is pending (the card the user approves must describe the history that is pushed); **no task branch holds unmerged work**: every task is `done`, or `ready` with no branch yet (a task in progress would be built on the old history).
- **Outcomes**: `Rebased {from, to, branches_moved}` · `UpToDate` · `Conflict {files, step, of}`: the rebase stops on a commit; the lead fixes the files and calls `mcp__orch__commit`, which, while a rebase is in progress, stages and runs `git rebase --continue` (no editor, no hooks) and returns the next outcome (another conflict or done). `rebase_base {abort: true}` aborts and restores the branch.
- **Specialist branches after a rebase**: their work is all merged (precondition), so on a successful rebase the core moves each specialist branch of the run whose head is contained in the pre-rebase integration head to the new integration head (`reset --hard` in its worktree, only when the worktree is clean; else the owner is told to call `sync_integration` before the next task). Event `git.rebase {from, to, branches_moved}`.
- Audited (`rebase`, actor `agent:<lead>`), like merges.

### 3.5 Pushing updates to the open PR
- `request_pr` while the run is `pr_open` asks for an **update** instead of a new PR: Inbox gate `action: "push_pr_update"`, title `<lead>: push <unpushed> commit(s) to PR #<n> (<branch>)`, or `<lead>: force-push PR #<n> rebased onto <base>` when `rewritten`. Payload `{run, branch, pr_number, url, commits: [{sha, subject}], force, lease_sha, base, worktree}` (`commits`: the new commits, or when `force` every commit of the rewritten history). `title` / `body` arguments are optional then and ignored.
- The run **stays `pr_open`** throughout: `pr_allowed` admits `pr_open`, asking the update does not move the run to `pr_gate`, and a rejected gate or a failed push leaves it in `pr_open` (unlike the PR gate, which goes back to `integrating`); the lead is told either way.
- On approval the core pushes the branch: a plain push when it fast-forwards; when rebased, **`git push --force-with-lease=<branch>:<lease_sha>`** where `lease_sha` is `pr.pushed_sha`, the PR head the app last pushed (not the head last seen on GitHub, which may hold someone else's commits). If the remote moved since (someone pushed to the PR), the push fails and is reported; never a bare `--force`. Then: audit (`pr_update`, with `force`), `pr.pushed` timeline event, an immediate PR check, and `ORCH: the user approved; pushed <n> commit(s) to PR #<n>. CI restarts; you will be told the result.` to the lead.
- Refused when nothing differs from the PR head ("nothing to push"), when a rebase is in progress, or when an update gate is already pending.
- Still one gate per push: the user approves every push, and the card says when it rewrites the PR's history (ADR-009).

### 3.6 Lead protocol
- Step 7 of the Lead prompt (`orch-team/src/compile.rs`) becomes: *"When every task is done and the integration branch passes its tests, call mcp__orch__request_pr. Once the PR is open the feature is not done: you are told when CI fails or the PR falls behind or conflicts with its base. Fix it: tasks as usual for CI; mcp__orch__rebase_base (once every task is merged) when it is behind or conflicts. Then mcp__orch__request_pr again to push. The feature is done when the PR is merged."*
- `rebase_base` is in the lead's tool list; the `commit` tool's description says it also continues a rebase.

### 3.7 UI (front, see [ux-brief.md](ux-brief.md))
- **Feature pane, card "PR and artifacts"**: the full URL is replaced by the short line `PR #<n>` + CI (a **filled dot**, green ok · red error · yellow running · grey waiting, then **`done/total`**) + position badge (+ `↑<n> to push` when `unpushed > 0`, `rebased` when `rewritten`, + `merged` / `closed` when not open). `PR #<n>` opens the PR in the browser (`openUrl`); the full URL is its tooltip. The CI tooltip lists **every job** with its own dot and state (each opens its link). A refresh button calls `pr_refresh`; "checked 2 min ago" is shown, and the line is marked stale when `error` is set.
- **Run state `pr_open`**: label "PR open" and a color in `runStateLabel` / `runStateColor` (sidebar, palette, Feature pane header, timeline). `runFinished` is unchanged (`done`, `failed`): the message box stays while the PR is open.
- **Timeline** texts for `pr.opened`, `pr.status`, `pr.pushed`, `pr.merged`, `pr.closed`.

## 4. Out of scope
- Solo sessions' PRs (no run): unchanged.
- A rebase while a task is in progress (refused, §3.4); a bare `--force` push.
- Merging the PR from the app, auto-merge, review requests or review comments.
- Re-opening runs already marked `done` before this change.
- Webhooks: polling only.

## 5. Non-functional
- **Security (ADR-009)**: agents still never push; every push is a user gate; a push that rewrites the PR is shown as such on the card and uses `--force-with-lease` pinned to the PR head the user saw; `rebase_base` only touches the integration worktree and specialist branches whose work is already merged.
- **Cost**: one `gh pr view` per open-PR run per minute, one `git fetch` of the base per 5 min, none while offline or on low battery.
- **Data (NFR-8)**: one migration (`team_runs.pr_number`, `team_runs.pr_status`); old rows read with `None`.
- **Tests**: `make ci` green. Unit tests: `RunState` moves (`pr_gate → pr_open → done`, `pr_open → integrating`, tasks allowed in `pr_open`); `PrStatus` parsing (merged, closed, CONFLICTING, BEHIND, UNKNOWN; job states → `done/total` and the dot's color, every precedence case); the watcher's transitions with a fake `gh` (one message per transition, none repeated after a reload, merged → done); `request_pr` in `pr_open` → update gate, refused when nothing to push, `force` + lease after a rebase; `rebase_base` on a fixture repo: clean rebase, conflict → commit continues → done, abort, refused while a task is in progress, specialist branches moved; force-with-lease refused when the remote moved. UI: vitest for the PR line / CI formatter and the new state label.

## 6. Acceptance (demo at the PR gate)
1. Approving a PR gate opens the PR and the run shows **PR open**, with `PR #<n>` + a grey or yellow dot `0/5` + up to date; the tooltip lists the 5 jobs.
2. A failing job turns the dot red (`5/5`), the lead is told once with the job names; after the fix is merged and the update gate approved, the commits are pushed to the same PR and CI runs again.
3. A commit landing on `main` shows `↓1 behind`; the lead is told, calls `rebase_base`; the line shows `rebased`, the gate card says force-push, and after the push the badge shows up to date, the feature's commits (and its task merges, kept by `--rebase-merges`) replayed on top of `main`. A conflicting change on `main` shows **conflicts**; the lead resolves them through `rebase_base` + `commit`.
4. Merging the PR on GitHub moves the run to **Done** within a minute; the lead is told; the team tools then refuse work.
5. Closing the PR without merging moves the run back to **Integrating**; `request_pr` opens a new PR.
6. Offline: no `gh` call, the line shows the last status as stale.
