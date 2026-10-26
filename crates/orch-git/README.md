# orch-git

Project registry and git service of the Rust core (SPEC §4.1 FR-101–111, J1, J11, J10 repair,
ADR-003, ADR-009). Plain functions over `git` and `gh` (no prompts, deadlines on network calls);
rows are kept in [orch-store](../orch-store/README.md). The agent runtime wraps them so worktrees in
use are protected ([agent-runtime `projects`](../agent-runtime/README.md)).

| Module | What | SPEC |
|---|---|---|
| `project` | `register(store, path)` (idempotent; a worktree registers its main checkout), `clone(store, url, parent, name, deadline)`, `removal_plan`, `remove` (J11) | FR-101, FR-109, FR-111 |
| `status` | `status(repo) -> RepoStatus`: branch, head, upstream, ahead/behind, staged/unstaged/untracked/conflicted, stash, last commit; `dirty()` | FR-102 |
| `fetch` | `FetchPolicy` (15 min, active within 7 days, battery ≥ 20 %, 120 s deadline), `Conditions` (`SystemConditions`: `pmset`, default route), `tick`, `fetch_now`, `due`, `blocked` | FR-103 |
| `worktree` | layout and naming, `ensure`/`complete` (add + lock + row, idempotent), `lock`, `list`, `inspect`, `others`, `common_dir`, `WorktreeLedger` | FR-105 |
| `cleanup` | `branches(repo, base)`, `worktrees(repo, rows, base)` with verdicts (ok / missing / unknown / prunable / main / external), `remove_worktree`, `delete_branch` | FR-104 |
| `diff` | `diff(worktree, against, max_bytes) -> Diff`: from the merge base to the working tree, untracked included, per-file status and counts, patch (bounded) | FR-107 |
| `gh` | `prs(repo)` with CI rollup, `runs(repo, branch)`, `available`; **gated** `push`, `create_pr` | FR-106, ADR-009 |
| `repair` | `check_links(wt)`, `plan_relocation(old, new, rows)`, `repair(repo, worktrees)` (targeted `git worktree repair` + re-lock) | J10, ADR-003 |
| `commit` | `commit_in_worktree(wt, CommitRequest, gitconfig) -> CommitResult`, `validate` | FR-702 gap, R10 |
| `fixtures` | (feature `fixtures`) `scratch_repo`, `commit_file`, `MemLedger`, `git` | tests |

## Safety rules

- **Unmerged work is never deleted silently** (FR-104, J11). `remove_worktree(…, Confirm::Safe)`
  refuses a worktree with uncommitted or untracked files, an operation in progress, or commits that
  the base does not have; `Confirm::DiscardUnmerged` is the user's explicit confirmation (force
  remove, `branch -D`). The main checkout is never removed. Locked worktrees are unlocked first; a
  worktree whose folder is gone is pruned. `project::remove` keeps every unconfirmed unmerged
  worktree and reports why; `.orchestrator/` in the checkout is never touched.
- **Commits for agents** (Claude Code 2.1.302 sandboxed git can't write the repository's git dir):
  the core commits in the agent's linked worktree only (never a main checkout, never while
  `index.lock` exists), with **hooks off** (`core.hooksPath=/dev/null`, `--no-verify`: a hook script
  is a file the agent can edit and would run outside the sandbox), `commit.gpgsign=false`,
  `core.fsmonitor=false`, literal pathspecs that must stay inside the worktree (no `..`, no
  absolute path, nothing under `.git`), a cleared environment, and the agent's generated
  `GIT_CONFIG_GLOBAL` (the user's name and email, no credential helper). Repo-local config applies
  as for any commit. No push here: `gh::push`/`create_pr` run only after the PR gate.
- **Fetch** uses the user's own credentials (the core runs outside the sandbox), never prompts
  (`GIT_TERMINAL_PROMPT=0`), and is killed at its deadline. Clone refuses option-like URLs.

## API notes

```rust,ignore
use orch_git::{project, status, worktree, cleanup, diff, fetch};
let p = project::register(&db, Path::new("~/dev/app"))?;          // orch_store::Project
let s = status::status(&p.path)?;                                   // RepoStatus
let row = worktree::ensure(&db, &p.path, &WorktreeKindSpec::Solo { slug: "fix".into() }, None, Some("a1"))?;
let infos = cleanup::worktrees(&p.path, &db.worktrees_of(&p.path)?, &p.default_branch)?;
let d = diff::diff(&row.path, &p.default_branch, 4 << 20)?;
fetch::tick(&db, &fetch::FetchPolicy::default(), &fetch::SystemConditions, orch_store::now_ms())?;
```

`WorktreeLedger` (get/put/rows of a repo) is implemented for `orch_store::Store` and for the agent
runtime's store; inside a running core, pass the runtime (its in-memory rows must stay current).

## Readings

- **Project id** = the folder name slugged (`demo`, then `demo-2`…). Registering a worktree path
  registers its main checkout. Default branch: `origin/HEAD`, else the checked-out branch.
- **Fetch interval** default 15 min ("periodically" is not given); `--all --prune`, no FETCH_HEAD.
- **"Active in the last 7 days"** = `last_active_at_ms`, bumped when the project is registered, an
  agent starts in it, or the core commits for one of its agents.
- **Battery**: on AC power fetch always runs; on battery only at ≥ 20 %. **Offline** = no default route.
- **Merged** = every commit of the branch is reachable from the base (`merge-base --is-ancestor`).
  Squash-merged branches look unmerged and need confirmation (the safe side).
- **Diff** compares the merge base with the working tree (uncommitted work included); untracked
  files are listed without content.
- **PR rollup**: CheckRun + StatusContext; `SKIPPED` is counted apart, anything not completed is pending.

## Tests

`cargo test -p orch-git`: naming, idempotent creation (relative links, lock, crash between add and
row), status (porcelain v2 and a real repo with upstream and stash), diff (rename, untracked,
truncation, base moved on), cleanup (orphans, refusals, confirmed discard, prune, branch deletion),
fetch (throttle, battery, offline, a real bare remote), register/clone/J11, joint move vs repo moved
alone (repair + re-lock, idempotent), commit (validation, partial/all, hooks never run, identity,
refusals), gh JSON parsing. No test calls GitHub.
