# Allow multiple PRs · Feature spec

> Feature `allow-multiple-prs` · Integration branch `orch/allow-multiple-prs/integration` · Base `main` (HEAD 540838fe)
> Touches: SPEC §6.2 (team run states), FR-106 (PRs via `gh`), FR-401 (Lead protocol), ADR-009 (only the app pushes), feature improve-end-of-feature (PR watcher, `rebase_base`, update push)
> Companions: [api-contract.md](api-contract.md) (core ↔ UI, MCP) · [ux-brief.md](ux-brief.md) (launch form, feature pane, cards)

## 1. Goal

A feature can address **several issues**. The user lists them on the feature and can **explicitly** choose **one pull request per issue** instead of one PR for the whole feature. The team still works on one integration branch (everything is built and tested together); at PR time the core builds, for each issue, a branch from the base that holds only that issue's work, and opens one PR per issue that closes it. Each PR is watched, updated, rebased and merged on its own; the feature is done when every issue's PR is merged.

The default does not change: one PR for the feature. Only the user switches a feature to one PR per issue; the lead cannot.

## 2. What the code does today

| Item | Today |
|---|---|
| Issues of a feature | None stored. The brief is free text; an intake start writes "From GitHub issue #12 · url" in it (`intake/start.rs:item_brief`). `Feature` has no issue field (`orch-store/src/teams.rs:19`). |
| PR of a run | One: `TeamRun.pr_url`, `pr_number`, `pr: PrStatus` (`teams.rs:47`). The PR head is the integration branch. |
| `request_pr` | Lead: one `push_pr` gate (`inbox.rs:1153`) → `open_pr` pushes the integration branch, `gh pr create`. When the run's PR is open, `request_pr` becomes the `push_pr_update` gate (`pr.rs:412`). |
| Watcher | `pr_tick` checks the single PR of each `pr_open` run (`pr.rs:143`); merged → `done`, closed → `integrating`. |
| Task merges | `merge` merges the specialist's long-lived branch `orch/<slug>/<node>-<n>` into the integration branch with message `Merge <branch>` (`team.rs:1856`). One branch per node, not per task: a branch holds the work of many tasks. |
| `rebase_base` | Rebases the integration branch onto `origin/<base>` with `--rebase-merges` (merge commits and their messages are kept, SHAs change) (`rebase.rs`). |

## 3. Scope

### 3.1 Issues and PR mode on the feature

- `Feature` gains `issues: Vec<IssueLink>` and `pr_mode: single | per_issue` (default `single`). One migration: `features.issues TEXT` (JSON), `features.pr_mode TEXT NOT NULL DEFAULT 'single'`.
- `IssueLink = {ref, title?, url?}`. `ref` is canonical:
  - GitHub issue of the project's repository: `#12` (input `12`, `#12`, `owner/repo#12` or an issue URL of **that** repository, parsed with `github_issue::parse_issue_ref`; another repository is refused).
  - Linear issue: `ABC-123` (upper-cased; `[A-Z][A-Z0-9]*-\d+`).
  - At most 10 issues; no duplicates.
  - `title` / `url` are filled best effort from the intake cache (the project's intake rows); no network call is needed to add an issue.
- Each issue has a **key** used in branch names: `12` for `#12`, `abc-123` for `ABC-123`.
- **Who sets them: the user only** (launch form, feature pane; `orchd`). No MCP tool changes issues or the mode.
  - `per_issue` needs at least 2 issues.
  - Adding an issue: any time the run is not finished.
  - Removing an issue: refused while it has a PR pending, open or merged.
  - Changing the mode: refused while any PR of the run is pending (gate) or open.
  - A change during a run is told to the lead in one ORCH turn (§3.7).
- A start from an intake item prefills `issues` with that item's ref (GitHub or Linear item; a repo doc has none).
- In `single` mode with issues listed, the PR body gets one `Closes <ref>` line per listed issue the lead's body does not already close (any `close[sd]?|fix(e[sd])?|resolve[sd]?` keyword before the ref, case-insensitive). Nothing else changes in `single` mode.

### 3.2 Tasks and commits carry their issue

- `Task` gains `issue: Option<String>` (a feature issue ref; column `tasks.issue TEXT`). `create_task` and `update_task` take `issue`; a ref not on the feature is refused. Allowed in both modes (shown on the board); it only changes PRs in `per_issue` mode.
- `merge {branch, task}`: the merge commit message gets the trailer `Orch-Task: <task id>` (both modes). In `per_issue` mode `task` is **required** (refused without it: the core could not tell which PR the merge belongs to).
- `commit {…, issue?}` (lead in the integration worktree): adds the trailer `Orch-Issue: <ref>`; a ref not on the feature is refused.
- Trailers survive `rebase_base` (`--rebase-merges` keeps messages).
- `merge` refuses the issue branches `orch/<slug>/issue/<key>` (they are not task branches).

### 3.3 Building an issue branch (core, `per_issue` mode)

For issue `I`, branch `orch/<slug>/issue/<key>`, a local branch of the project repository written by the core (outside the sandbox; no worktree):

1. **Fork point** `F` = `git merge-base <integration> origin/<base>` (the base the integration branch currently sits on).
2. **Commits**: the first-parent commits of `F..<integration>`, oldest first. Each is attributed:
   - a merge commit with `Orch-Task: n` → task `n`'s `issue`;
   - a commit with `Orch-Issue: r` → `r`;
   - anything else (lead docs, merges without a task, an untagged task) → **shared**.
   A task's issue is read at build time, so re-tagging a task (`update_task`) changes the next build.
3. **Replay** onto `F` the commits attributed to `I` or shared: each commit `C` is applied as its own change, `diff(C^1, C)` (for a task merge, that is exactly what the task brought), with `git merge-tree --write-tree --merge-base <C^1> <head> <C>` then `git commit-tree` with `C`'s author and message. A commit whose change is already there (empty) is skipped.
4. **Conflict**: the build stops and nothing is written. The core returns the commit, its task (handle, node) and the files. Usual cause: a task of `I` builds on code from another issue's task. The lead fixes it by re-tagging tasks (`update_task issue`), or asks the user (`ask_user`) to group the issues differently or to go back to one PR.
5. **Update** of a branch whose PR is open:
   - same fork point as the PR's last build → the rebuilt tree is added as **one commit on top of the PR head** (`Update for <ref>: <task handles>`), a fast-forward push. Nothing is pushed when the tree did not change.
   - fork point changed (after `rebase_base`) → the full rebuild replaces the branch: force push with lease, exactly as the single PR after a rebase (lease = the PR's `pushed_sha`).
- The build runs inside `request_pr`, so the gate the user approves shows the exact branch head and diff (`base...issue branch`).

Shared work (untagged commits, the feature docs under `.orchestrator/features/<slug>/`) goes into **every** issue PR: each PR then builds and tests on its own. Identical changes merge cleanly when the PRs land one after the other.

### 3.4 Requesting, opening and updating the PRs

- `request_pr {issue, title, body, draft?}`: in `per_issue` mode `issue` is required (a feature issue). The core builds the branch (§3.3) and then:
  - no open PR for that issue → a `push_pr` gate for that issue (one gate per issue; the user approves each on its own);
  - its PR is open → a `push_pr_update` gate for that PR (`title`/`body` ignored), normal or force push (§3.3 step 5);
  - nothing to push → refused with "PR #n is up to date".
  - Refused while a gate of the same issue is pending. Gates of different issues can wait side by side.
- `request_pr` in `single` mode: unchanged (an `issue` argument is refused: "this feature has one PR").
- `open_pr`: pushes the issue branch, `gh pr create --base <base> --head orch/<slug>/issue/<key>`, the body with `Closes <ref>` added when missing (§3.1 rule).
- `decision_patch` and the gate's diff summary use the gate's own branch.
- Stale gate: the gate stores the integration head and the built head; either moving before the approval makes it stale ("call request_pr again"), as today.

### 3.5 The run's PRs, watched one by one

- `TeamRun` gains `prs: Vec<RunPr>` (column `team_runs.prs TEXT`, JSON), one per issue that ever had a PR in `per_issue` mode:
  `RunPr = {issue, branch, number, url, pr: PrStatus?, fork_sha, built_sha}` (`fork_sha` = `F` of its last build, `built_sha` = the head last built and pushed).
- In `single` mode the existing fields (`pr_url`, `pr_number`, `pr`) stay the single source; `prs` is empty. The UI reads one list (`prs_view`, api-contract §2) built by the core for both modes.
- `PrStatus` per issue PR as today, except: `unpushed` = commits on the integration branch attributed to the issue or shared since its last build; `rewritten` = the fork point moved since the last build.
- `pr_tick` checks every open PR of every run (each with its own fetch throttle). Messages to the lead name the issue: `ORCH: CI fails on PR #34 (issue #12): …`, `PR #34 (issue #12) is behind main by 3 commits …`.
- **Run state** in `per_issue` mode:
  - `pr_gate` while a `push_pr` gate is pending and no PR is open; `pr_open` while at least one issue PR is open;
  - a PR merged → the lead is told `PR #34 (issue #12) was merged; N issue(s) left: …`; the run moves to `done` when **every** issue of the feature has a merged PR (Inbox info "Feature <slug> merged", as today);
  - a PR closed unmerged → the lead is told; the run stays `pr_open` while another PR is open, else goes back to `integrating`. The lead may request that issue's PR again (a new PR replaces the closed one in `prs`).
  - when the last open PR merges but issues without a PR remain → `integrating` (work left).
- A pending `push_pr_update` gate of a PR that merges or closes is withdrawn (that PR's only).
- `pr_refresh` checks every open PR of the feature's run.

### 3.6 `rebase_base` and the other lead tools

- `rebase_base`: unchanged on the integration branch. Its preconditions also look at every pending gate of the run. After it, each open issue PR is `rewritten`: the lead calls `request_pr {issue}` for each, which rebuilds and force-pushes with lease (§3.3).
- `team_status` lists the feature's issues, the mode and, per issue, its tasks and PR (number, state).
- `ask_user` / `read_issue` unchanged.

### 3.7 Lead protocol and prompts

- The lead's run prompt (`orch-team` compile) gets a "Pull requests" line: `One PR for the feature` (listing the issues to close when there are some) or `One PR per issue: #12, #15. Give every task its issue (create_task issue); tag your own commits (commit issue); untagged work goes into every PR. Ask for each PR with request_pr {issue} when that issue is done; the feature is done when every issue's PR is merged.`
- The user changing the issues or the mode mid-run sends the lead one ORCH turn with the new setting and, for a switch to `per_issue`, the merged tasks that have no issue yet (to tag with `update_task`).
- Specialists' prompts do not change.

### 3.8 UI (front; see [ux-brief.md](ux-brief.md))

- Launch form: an **Issues** field and a **Pull requests** choice (`One PR` / `One PR per issue`, the second enabled from 2 issues).
- Feature pane: issues and mode shown and editable under the rules of §3.1; the "PR and artifacts" card shows **one PR line per issue** (`PrLine` reused per entry) with the issue and its state (`no PR yet`, gate pending, open with CI and position, merged, closed).
- Mission run row and run inspector: the PR summary for several PRs.
- Decision cards (`push_pr`, `push_pr_update`): the issue in the title and the issue branch.
- Board / task inspector: the task's issue chip.

### 3.9 `orchd`

- `launch --issue <ref>… --pr-mode single|per_issue`, `feature-prs <feature> [--issue …] [--pr-mode …]`, and `runs` printing `prs`.

## 4. Out of scope

- Stacked PRs (a PR based on another issue's branch). A dependency between issues is handled by re-tagging, grouping or one PR.
- Grouping several issues in one PR (other than the whole feature in `single` mode).
- One PR per issue for solo sessions (no integration branch).
- Starting one feature from several intake items at once (the user types the other refs).
- Closing / commenting the issues ourselves (GitHub and Linear close them from the `Closes` lines).
- Fetching issue titles over the network in the form.

## 5. Non-functional

- `single` mode behaves exactly as today (all existing PR tests pass unchanged); the only visible difference is `Closes` lines when issues are listed.
- The build never touches a worktree, never runs hooks, never pushes (only `open_pr` / `push_pr_update` push, after the gate).
- Migrations are additive; old runs read with `prs` empty and `pr_mode = single`.
- Tests: the build (attribution, shared, empty skip, conflict, update commit, rebuild after rebase), gates per issue, the watcher and run states with 2 PRs, the MCP refusals, UI pure helpers (`prs` summary copy) with vitest.

## 6. Acceptance (demo)

1. Launch a feature with issues `#1`, `#2` and **One PR per issue**. The lead's prompt says so; the board shows each task's issue.
2. Two tasks tagged `#1` and `#2`, plus the lead's spec commit (shared), merged into the integration branch.
3. `request_pr {issue: "#1"}` and `{issue: "#2"}`: two gates, each with only its issue's diff plus the shared docs. Approve both: two PRs from `orch/<slug>/issue/1` and `/issue/2`, bodies with `Closes #1` / `Closes #2`.
4. The feature pane shows two PR lines with their CI. A fix task tagged `#2`, merged, `request_pr {issue: "#2"}` → an update gate pushing one commit to PR 2 only.
5. Merge PR 1 on GitHub: the lead is told one issue is left; the run stays `PR open`. Merge PR 2: the run is `done`.
6. A task of `#2` that edits a file created by a `#1` task: `request_pr {issue: "#2"}` is refused with the conflicting task and files.
7. A feature in `single` mode with issues `#1`, `#2`: one PR, body with `Closes #1` and `Closes #2`.
