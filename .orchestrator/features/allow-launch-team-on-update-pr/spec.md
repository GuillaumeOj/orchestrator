# Allow to Launch a Team on Update PRs · Feature spec

> Feature `allow-launch-team-on-update-pr` · Integration branch `orch/allow-launch-team-on-update-pr/integration` ·
> Team: lead, back, front, ui, ux.
> Companions: [api-contract.md](api-contract.md) (back ↔ front), [ux-brief.md](ux-brief.md) (ux → ui → front).
> Code base: **origin/main** (#88). The integration branch starts at #82; it is rebased on main before any code task.

## 1. Problem

Updater PRs (Renovate, Dependabot, or any PR opened by someone else) sometimes need a change to pass CI: a renamed
API, a lint rule, a lockfile, a snapshot. Today a team can only start from a **new** branch (`orch/<slug>/integration`
cut from the base) and always ends with `gh pr create`: there is no way to work **on the PR's own branch** and push the
fix to it.

The user wants to pick such a PR in the project overview's **Pull requests** list, launch a team on it, let the team
fix it on that branch, and push the result to the same PR.

### 1.1 What the code does today (read on main)

- The integration branch is never stored: it is rebuilt as `orch/<slug>/integration` in `Feature::integration_branch`
  (orch-store `teams.rs:45`), `FeatureCtx::integration_branch` (orch-team `compile.rs:138`) and the Integration
  worktree layout (orch-git `worktree.rs:57`). ~25 callers use it (members branch from it, merge guard, sync, rebase,
  PR lookup, prompts, UI copy). Only the worktree row stores the real `branch` (and `base`).
- Launch (`team.rs:449`) creates the worktree from the **local** base ref, nothing fetched.
- `request_pr` without an open PR → `push_pr` gate → `gh::push` + `gh pr create` (inbox.rs:1296). With an open PR
  (`pr_open` + `run.pr` checked) → `push_pr_update` gate → fast-forward push or `--force-with-lease` (pr.rs:573–660).
  **Pushing to an existing PR already works once a run is `pr_open`.**
- The PR watcher reports CI, merged/closed, behind/conflicts, but never a head moved by someone else; a closed PR
  moves the run back to `integrating` and the next `request_pr` would open a **new** PR.
- The project PR list (`gh pr list --json number,title,headRefName,baseRefName,state,url,isDraft,reviewDecision,statusCheckRollup`)
  has no author, no fork flag, no head sha, and no action.

## 2. Goals

1. **Launch from the PR list.** Each open PR row of Project › Pull requests gets **Launch team…**. It opens the usual
   launch dialog prefilled: title, slug, brief (PR title, link, branches, author, the failing checks and their
   failure excerpts), team, and the PR shown as the run's target ("Works on PR #42 · `renovate/foo` → `main`").
2. **The team works on the PR branch.** The feature's integration branch **is the PR head branch** (`renovate/foo`),
   checked out in the usual integration worktree from `origin/<head>` (fetched at launch). Members branch from it and
   merge into it as usual. The feature is **linked** to the PR (number, url, head, base, head sha at launch).
3. **The team pushes to the PR, never opens one.** The lead's `request_pr` on a linked feature asks the user to
   **push to PR #42** (same gate card, with only the team's commits listed); approved → push to `origin/<head>` with a
   lease on the head the app last saw; the run becomes `pr_open` on PR #42 and the existing watcher takes over (CI,
   merged → done, updates through `push_pr_update`).
4. **Safe with a moving or closed PR.** A head moved outside the app is told to the lead, who can bring it in
   (`rebase_base` onto the PR head). A linked PR that is closed or merged is never replaced by a new PR.
5. The UI names the real branch everywhere it names the integration branch today (dialog, Feature pane, delete dialog).

## 3. Non-goals

- **Fork PRs** (`isCrossRepository`): every push goes to `origin`. The row shows the action disabled with the reason.
- **Per-issue PR mode** on a linked feature (refused at launch; the dialog hides the PR mode control).
- No change to how updater bots behave; no Renovate/Dependabot-specific API (no "rebase" checkbox, no bot commands).
  Renovate usually stops rebasing a branch someone else committed to; that is its business.
- No automatic launch (e.g. "on every failing Renovate PR"): starting stays a dialog the user submits.
- No GitHub Enterprise: `github.com` / `origin`, as everywhere else.
- No other entry point than the project PR list in this feature (the Feature pane, Work browser and Mission control
  keep their current launches).

## 4. Decisions (lead's; the user can overturn them at spec approval)

- **D1 · Feature docs stay off the PR.** The lead still writes and commits `spec.md` (the spec gate needs it), but
  on a linked feature the lead removes `.orchestrator/features/<slug>/` in a last commit before `request_pr`, and
  `request_pr` is refused while the PR diff (`origin/<base>...HEAD`) still contains that folder. The PR history keeps an
  add/remove pair (gone on a squash merge). The lead protocol says so for linked features.
- **D2 · The linked branch is never deleted on the remote**, and locally only when its head is in `origin/<head>`
  (pushed). Deleting a linked feature, pruning or removing the project keeps an unpushed local PR branch.
- **D3 · One active feature per PR.** A PR already linked to a feature with a run that is not done/failed shows
  **Open feature** instead of Launch team. A finished feature on the same PR can be relaunched (existing-feature path).
- **D4 · Draft PRs are allowed**; closed/merged PRs are not listed (the list is open PRs only).
- **D5 · Same lifecycle.** A linked run follows the usual states (spec → spec gate → planning → … → pr gate → pr open
  → done); no shortcut state. The spec gate stays: it is where the user sees what the team will change on someone
  else's PR.

## 5. Behaviour

### 5.1 PR list (back + front)

- `gh pr list` also asks for `author`, `isCrossRepository`, `headRefOid`, `headRepositoryOwner`; each PR row gains
  `author {login, is_bot}`, `cross_repo`, `head_sha` and `feature` (the linked feature on this project, if any:
  id, slug, run status) — api-contract §1.
- Row action: **Launch team…** (enabled), **Open feature** (D3), or disabled **Launch team…** with the reason
  ("Fork PR: the app pushes to origin only").

### 5.2 Prefill (back)

`pr_prefill(project, number)` returns the launch prefill (shape of intake's `prefill_forms`) plus `pr` (the link to
send back) and `origin` (the dialog's "From PR #42" chip). It runs `gh pr view` (fresh head sha, state, checks) and,
for each failing check run of GitHub Actions, the failed log tail (`gh run view <run> --log-failed`, last 60 lines per
job, 4 jobs, 12 KB in all; annotations when there is no log). Slug: `pr-<n>-<kebab title>` (unique, ≤ 40). Team: the
project's default team as for intake. Brief template in api-contract §2. A closed / merged / fork PR → error code.

### 5.3 Launch (back)

`LaunchSpec` gains `pr: Option<PrLink>` (api-contract §3). With `pr` set:

1. Refused: `issues` non-empty with per-issue mode, a fork PR, a PR linked to an active feature (D3), the head branch
   checked out in another worktree (the user's main checkout included: "renovate/foo is checked out in ~/dev/shop;
   switch that checkout to another branch first").
2. Fetch `origin <head>`; check the PR is still open and get its head sha (from the link, refreshed by the fetch).
3. Feature: new columns `branch` (the integration branch name, NULL = `orch/<slug>/integration`) and `pr_link` (JSON).
   `base` = the PR base. Every derivation of the integration branch reads `branch` when set (`Feature`, `FeatureCtx`,
   `app_state.features[].branch`, orchd CLI); the worktree layout takes the branch explicitly.
4. Integration worktree at the usual path `<proj>.worktrees/<slug>/integration`, branch `<head>`:
   - no local `<head>` → `git worktree add -b <head> <path> origin/<head>` (upstream set to `origin/<head>`);
   - a local `<head>` already at or behind `origin/<head>` → fast-forward it to `origin/<head>`;
   - a local `<head>` with commits not on origin → refused ("local renovate/foo has 2 commits that PR #42 does not
     have; push or delete them first").
   The worktree row keeps `base` = the PR base (diffs, cleanup, PR base stay right); the start ref is separate.
5. Run as usual (lead started, brief queued, run `spec`). The lead prompt says: "Pull requests: this feature works on
   PR #42 (`renovate/foo` → `main`, opened by renovate[bot]). The integration branch is `renovate/foo`. Never open a
   pull request; `request_pr` pushes to PR #42. Remove the feature docs before `request_pr` (D1)."

### 5.4 Merge, sync, rebase (back)

- Merge guard: the source must start with `orch/<slug>/` and must not be the feature's integration branch (whatever
  its name) nor `issue/*`. Specialists' branches stay `orch/<slug>/<node>-<n>`.
- `policy/rt.rs` protects the real integration branch name.
- `rebase_base` gains `onto: "base" | "pr_head"` (default `base`). `pr_head` (linked features only) fetches
  `origin/<head>` and rebases the integration branch on it (same conflict / continue / abort / member-reset machinery);
  when the local branch is behind only (no team commit yet), it fast-forwards.

### 5.5 Push to the PR (back)

- `request_pr` on a linked feature without `pr_open`: refused when the docs folder is in the diff (D1), when there is
  no commit beyond the link head; else a `push_pr` gate with `linked: {number, url}`, `diff_summary` over
  `origin/<head>...HEAD` (the team's commits only) and `lease` = the PR head last seen (link head, or the last fetch).
- Approved → `gh pr view` (still open, head still the lease; a moved head refuses with "PR #42 moved since the
  request: rebase on it (`rebase_base` onto `pr_head`) and ask again") → `push_with_lease(HEAD, <head>, lease)` (a
  plain fast-forward when the lease is an ancestor) → `pr_opened(linked url)` → `pr_open`, `pushed_sha` = HEAD. **No
  `gh pr create`.** Audit `pr_push_linked`.
- In `pr_open` the existing `push_pr_update` path is used unchanged.
- Closed linked PR → run `integrating`, the lead told "PR #42 was closed outside the app; ask the user", and
  `request_pr` refused while the link's PR is closed. Merged → `done` as today.

### 5.6 Watcher (back)

- Linked runs are checked by `pr_tick` from launch (not only in `pr_open`), so the Feature pane shows the PR's CI and
  state from the start. Before `pr_open`, changes are reported to the lead only for closed/merged and a head moved.
- **Head moved outside the app** (`head_sha` ≠ last seen and ≠ `pushed_sha`, not our push): one ORCH turn to the lead
  ("PR #42's head moved to abc1234 (pushed by someone else, e.g. the updater). Bring it in with `rebase_base` onto
  `pr_head` before asking for the push."), and `run.pr.foreign_head = true` for the UI. Reset when the branch contains
  it again.

### 5.7 Delete and cleanup (back)

D2: `remove_worktree` / feature delete / prune / project removal never run `git branch -D` on a linked branch whose
head is not in `origin/<head>`; they keep it and say so ("kept renovate/foo: 1 commit not pushed to PR #42").

### 5.8 UI (front, after ui)

- Project › Pull requests rows: author (bot mark), CI, the action (5.1). **Load PRs** unchanged.
- Launch dialog, PR variant: "From PR #42" origin chip; a read-only **Works on** line (PR link, head → base, author)
  replacing Base; no PR mode control; copy "Checks out renovate/foo" instead of "Creates orch/…/integration";
  the prefilled brief editable; errors from launch inline (5.3.1, 5.3.4).
- Feature pane: header / PR card shows "PR #42 · renovate/foo → main" from launch (state, CI, linked, foreign head
  warning), the real branch name in place of the hard-coded one (`FeaturePane.svelte:73`).
- Gate card for a linked push: "Push 3 commits to PR #42 (renovate/foo)" instead of "Open a pull request".
- Delete-feature dialog: the real branch and D2's rule in the checkbox copy.

## 6. Acceptance

1. A same-repo open PR in Project › Pull requests → Launch team… → dialog prefilled with failing check excerpts →
   Launch → the integration worktree is on `renovate/foo` at `origin/renovate/foo`; members branch from it.
2. The run's spec gate, tasks and merges work as for any feature; the lead's `request_pr` after removing the docs
   shows "Push n commits to PR #42"; approve → the commits are on `origin/renovate/foo`, no new PR exists, the run is
   `pr_open` on #42; CI and merge are followed; merge → `done`.
3. Fork PR: action disabled with the reason. A PR with an active linked feature: Open feature.
4. A head pushed by someone else before the push: lead told; the approval with the old lease is refused, never
   overwrites; `rebase_base onto pr_head` then a new request pushes.
5. Closed linked PR: run back to `integrating`, no `gh pr create` possible.
6. Deleting a linked feature with unpushed commits keeps the local branch; nothing is ever deleted on the remote.
7. Unlinked features behave exactly as before (all existing tests pass; `branch` NULL everywhere).

## 7. Work split (planned after approval)

- **ux** X1: flows, copy, states (notes `docs/design/allow-launch-team-on-update-pr-design-notes.md`) → hands off to ui.
- **ui** U1: FigCraft frames (PR list rows, dialog PR variant, Feature pane linked PR, gate card, delete dialog) and
  exports under `design/`.
- **back** B1 store + launch + branch derivation + guards (5.3, 5.4 guard, 5.7); B2 PR list + prefill (5.1, 5.2);
  B3 push gate, watcher, `rebase_base onto pr_head`, lead prompt (5.4 rebase, 5.5, 5.6).
- **front** F1 (after U1, B1, B2): PR list, dialog, Feature pane, gate card, delete dialog.
- **ux / ui** review F1.
