# add-missing-commits: land what PR #78 left behind

## Goal
PR #78 (`orch/allow-multiple-prs/integration` → `main`, run run-a853e2efc2) was merged
without three commits from the team's branches. This feature lands those three commits, and
nothing else, in one follow-up PR.

## How the missing commits were found
- `main` is at `8e9b54da` (Merge PR #78). Its second parent is `36cb78a1`, and the two have
  the same tree, so #78 shipped exactly `36cb78a1`.
- `git log --cherry-pick --right-only main...<branch>` was run for
  `orch/allow-multiple-prs/{back-1,front-1,ui-1,ux-1,integration}`, then each branch's tree
  was diffed against `main` (`git diff main <branch>`), because the rebase rewrote the SHAs.
- `ui-1` and `ux-1` have nothing missing: their tips are `36cb78a1`.
- `front-1` lists 40 commits that are not cherry-equivalent to `main`. These are the copies
  from before `rebase_base`, and `main` holds all of them in rebased form. Its tree differs
  from `main` by one line only, which is the commit below. So all of those 40 are dropped.
- None of the commits involved carries an `Orch-Task` or `Orch-Issue` trailer. The feature
  ran in one-PR mode, and the merge commits were rewritten by the rebase, so there are no
  trailers to keep.

## The missing commits

| # | SHA | Branch / owner | Task | Subject | Files |
|---|-----|----------------|------|---------|-------|
| 1 | `06947016` | integration / lead | none (CI fix for T-413's test) | fix(allow-multiple-prs): no lost PR when two issue PRs open at once | `crates/orch-store/src/teams.rs`, `crates/orch-store/src/migrations.rs`, `crates/agent-runtime/src/pr_issue_tests.rs`, `crates/agent-runtime/src/feature_prs_tests.rs` |
| 2 | `ce70bdf2` | back-1 / back | T-421 | fix(allow-multiple-prs): review fixes: docs of issue_rows/row_ref, reuse row() helper in test | `crates/agent-runtime/src/issues.rs` |
| 3 | `44143ed7` | front-1 / front | T-420 | docs(allow-multiple-prs): issue chip sits next to the card title | `ui/src/components/TaskIssueChip.svelte` |

Why each one was missed:
1. **`06947016`** was committed on the integration branch at 12:31, after the PR head
   `36cb78a1` had been pushed. It fixes the CI failure in
   `two_issues_get_their_own_prs_updates_and_the_run_ends_when_both_merge` and a real race:
   `upsert_run` rewrote `prs` from the caller's stale copy of the run, so when two issue PRs
   opened at once, one of them could be dropped. It never reached the PR, so **`main` has
   both the flaky test and the race**. This is the commit that matters most.
2. **`ce70bdf2`** was committed on back-1 at 12:05, after T-421 had been merged at
   `993a949b`. These are review fixes (a doc comment that sat on the wrong function, and a
   test that now uses the existing `row()` helper), and they were never merged.
3. **`44143ed7`** was committed on front-1 after the T-420 head that was merged (`7f3a040c`,
   rebased to `818a2412`). Only its comment changed, and front-1 was never merged again.

Main's tree equals `36cb78a1`, the parent of commits 1 and 2 (commit 3's parent is the
pre-rebase `7f3a040c`, whose tree differs from `main` in many files, but the patch is one
comment line that `main` has unchanged), so each one applies to `main` without conflict.

## What changes
- Port the three commits, in the order above, onto the integration branch once it is
  updated from `main`. Each one keeps its author and its full message.
- No other code changes: no new feature, no refactor, nothing in #78 reverted.

## Acceptance
- `git diff main <PR head>` equals the union of the three patches above.
- `cargo fmt --check` and `make clippy` (`--workspace --all-targets`) are clean.
- `cargo test -p orch-store` and `cargo test -p agent-runtime` pass, in particular
  `migrations` (the stale-copy check), `pr_issue_tests` and `feature_prs_tests`. The
  pty/socket/FSEvent tests the local sandbox can't run are named in the report, and CI
  judges them.
- UI: `biome check` and `svelte-check` are clean, and the vitest suite that covers `TaskIssueChip`
  (`ui/test/issues-ui.test.js`) passes.
- The PR description lists each recovered commit and why it was missed.

## Out of scope
New features, refactors, and reverting anything in #78.
