# Remove Worktrees Created Outside the App · Feature spec

> Feature `fix-remove-worktrees-outside-the-app` · Integration branch `orch/fix-remove-worktrees-outside-the-app/integration` · Base `main` @ f841c01 (PR #61 included) · back + front, no design pass (the existing worktree table is reused, copy only)

## 1. Problem

The project pane (`ui/src/panes/ProjectPane.svelte`) lists the project's worktrees. The user has worktrees the app did not create, for example Claude Code's `<repo>/.claude/worktrees/<name>`, or a `git worktree add` of their own. Many of their folders are already gone from disk; git still lists them as `prunable`. On the user's repo today git lists 13 `.claude/worktrees/*` entries plus 2 hand-made ones (`fix-self-kill-on-reconcile`, `p3-ecosystem-v2`), and reports them `prunable` (seen from the lead's sandbox, which can't read those folders, so some may still be on disk).

The user can't get rid of them from the app:

- **Folder present, outside the sibling folder** (verdict `external`): listed only under a collapsed `<details>` line, with no Remove button.
- **Folder gone, outside the sibling folder** (verdict `prunable`): listed in the main table **with** a Remove button. Clicking it always fails with `… is not in <root>: the app only removes its own worktrees` (`agent-runtime/src/projects.rs:163`).
- They are never safe to remove: `safe_to_remove` excludes `external` (`orch-git/src/cleanup.rs:191`), so even a merged, clean one would get the "Discard unmerged work" dialog.
- There is no way to clean all the gone ones at once: each one is a click and a dialog.

**Goal:** every worktree of the project except the main checkout can be removed from the pane, whoever created it, and every worktree whose folder is gone can be pruned in one action. Nothing the user can't get back is lost without the explicit confirmation FR-104 asks for.

## 2. Rules

**Who may be removed.** Any worktree git lists for the project's repository, plus the app's live rows (`missing`), except:

- the main checkout (always refused);
- a worktree in use by an agent that is neither archived nor killed (unchanged: `in_use_by`, refused with `agent X uses this worktree: archive or kill it first`).

A path that is neither one of git's worktrees of this repository nor a live row of the app is refused (`… is not a worktree of <project>`). The core never deletes an arbitrary folder: the "stray folder" `remove_dir_all` (`cleanup.rs`, not a git worktree any more) stays limited to the sibling folder.

**The app's own worktrees (in the sibling folder)**: unchanged. The branch is deleted with the worktree, `-D` only with discard.

**Worktrees outside the sibling folder (`outside: true`) are the user's**, and their branch is the user's too:

- **The branch is deleted only when it is merged into the project's default branch.** An unmerged branch is **kept**, even with discard, and the outcome says so (`kept_branch`). Its commits stay reachable, so they don't count as work that would be lost.
- So the confirmation is needed only for what really goes: uncommitted or untracked files, an operation in progress, or a detached HEAD with commits the base doesn't have. `safe_to_remove` follows that rule (an outside worktree that is clean, on a branch, with its folder present or gone, is safe).
- **A locked one needs the confirmation.** A lock usually means another tool (Claude Code, an IDE) is using it. The pane shows the lock's reason, and the dialog says it. With discard, the core unlocks and removes (as it does today for the app's own).

**Prune gone worktrees (bulk).** One action on the pane: every listed worktree whose folder is gone (verdict `prunable` or `missing`), wherever it is, not in use by a live agent. For each: git's entry goes (`git worktree prune` or `remove`), the app's rows are marked `Removed`, and the branch is deleted only when merged into its base. Unmerged branches are kept and reported, never `-D`, so this action needs no discard, only one confirmation. Worktrees in use are skipped and reported. The project's topic is touched once at the end.

## 3. Scope

### Core (`back`)

- **B1 · List:** each worktree entry carries `outside: bool` (its canonical path is not under the project's worktree root) and `lock_reason: string | null` (git's `locked <reason>` line; `locked` stays). Verdicts don't change: `external` is still "outside, folder present"; an outside worktree with its folder gone stays `prunable`, now with `outside: true`.
- **B2 · `safe_to_remove`:** per §2, an outside worktree is safe when it is not locked, has no dirty file, no operation in progress, and is on a branch or its detached HEAD is merged. The sibling folder's rule doesn't change.
- **B3 · Remove (`worktree_remove`):** drop the "only removes its own worktrees" refusal. Apply §2: refuse paths that are neither a git worktree of the repo nor a live row; for outside worktrees, keep an unmerged branch (no `-D`) and skip the "not merged" check when the tip stays on a kept branch; refuse a locked outside worktree without discard (`… is locked (<reason>): needs confirmation`); never `remove_dir_all` outside the root. `RemoveOutcome` gains `kept_branch: string | null`.
- **B4 · Prune (`worktrees_prune`):** a new command per §2 and `api-contract.md`. Through the same runtime path as B3 so in-use checks, row marking, the audit (`worktrees_pruned`) and the topic stay in one place.
- **B5 · Docs:** the doc comment on `Runtime::remove_worktree` and the `WorktreeVerdict::External` comment follow the new rules.
- **Tests** (`orch-git/src/cleanup.rs`, `agent-runtime/src/projects.rs`, `orch-app/src/core.rs`; the existing `projects_live_in_the_store_and_leave_per_j11` assertion `contains("only removes its own")` is replaced):
  - an outside worktree with its folder present, clean and merged: listed `external`, `outside: true`, `safe_to_remove: true`; removed without discard; its merged branch is deleted; the next `project_detail` doesn't list it;
  - the same with an unmerged branch: removed without discard, branch kept, `kept_branch` set;
  - an outside worktree with dirty files: refused without discard, removed with it;
  - an outside worktree with its folder gone (`prunable`, `outside: true`): removed, gone from the list;
  - a locked outside worktree: `lock_reason` listed, refused without discard, removed with it;
  - `<repo>/.claude/worktrees/x` (nested in the main checkout) is removed like any outside worktree, and the main checkout is still refused;
  - a path that is not a worktree of the repo (a plain folder elsewhere) is refused and left on disk;
  - `worktrees_prune`: with a mix (app worktree folder gone and merged, app worktree folder gone and unmerged, outside gone, outside present, one in use), it removes the gone ones, keeps the unmerged branch, skips the one in use, leaves the present one, and touches the topic.

### UI (`front`)

- **U1 · One table, two groups.** The Worktrees card lists the sibling folder's worktrees (as today), then the outside ones under a sub-heading **Not created by the app** (`outside: true`, verdict `external` or `prunable`), with the same columns and actions. The collapsed `<details>` goes. Diff (±) only when the folder exists (not for `prunable` / `missing`).
- **U2 · State copy:** `prunable` and `missing` read `folder gone`; `external` reads `yours`; a locked worktree adds ` · locked: <reason>` (or ` · locked` when there is no reason). The card's subtitle keeps "unmerged work is never deleted without confirmation".
- **U3 · Remove dialogs:** per row, same `worktree_remove` call (`deleteBranch: true`).
  - Safe: `Remove worktree <branch>?` / for an outside one: `<path>. Its branch is deleted if merged into <default>, kept otherwise.`
  - Not safe, outside: `Remove worktree <branch>?` / body says what goes (uncommitted files, operation in progress, lock and its reason) and that the branch is kept if unmerged, button **Discard and remove**, `discard: true`.
  - Not safe, sibling folder: unchanged.
  - Toast: `Removed <path>`, plus ` · kept branch <name> (unmerged)` when `kept_branch` is set.
- **U4 · Prune:** in the card header, a **Prune N gone…** button when at least one row has its folder gone and no `in_use_by`. Confirm: `Prune N gone worktrees?` / `Their folders are already gone. Their git entries go, and their branches when merged; unmerged branches are kept.`, button **Prune**. Calls `worktrees_prune`, drops the removed paths from the list at once (like the single removal, with `seq++`), toast `Pruned N worktree(s)` + ` · kept K unmerged branch(es)` + ` · skipped S in use`.
- **Tests** (`ui/test/`, through the real dialog like PR #61, `ProjectPane` with `Dialogs`): an outside row has a Remove button and removes with the right args; an outside `prunable` row too; the toast names a kept branch; a locked row shows its reason and asks **Discard and remove**; no Diff button on a gone row; **Prune N gone…** appears with the right N (rows in use not counted), calls `worktrees_prune`, removes the rows and shows the toast; it doesn't appear when nothing is gone.

### Docs (`lead`)

`docs/SPEC.md` FR-104 gets one sentence: worktrees outside the sibling folder can be removed too; their unmerged branches are kept.

Out of scope: J11 project removal (it still only cleans the sibling folder), branch-only cleanup of `claude/*` branches with no worktree, the agent delete dialog, any new design.

## 4. Acceptance

1. In the project pane, a worktree created outside the app (`.claude/worktrees/x` or a hand-made `git worktree add`) can be removed, folder present or gone. It leaves the list and doesn't come back.
2. Removing an outside worktree never deletes an unmerged branch; the toast says the branch was kept.
3. Uncommitted files, an operation in progress or a lock need the explicit **Discard and remove**.
4. **Prune N gone…** clears every gone worktree not in use in one confirmation, keeping unmerged branches.
5. The main checkout and worktrees used by a live agent are still refused; a path that isn't a worktree of the repo is refused and untouched.
6. `cargo test` (with `ORCH_TEST_TMP`), `cargo clippy`, `bun run test`, `bun run check` and `bun run lint` pass.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-1 Core: `outside` / `lock_reason`, outside removal rules, `worktrees_prune` | back | — | `orch-git/src/{cleanup,worktree}.rs`, `agent-runtime/src/projects.rs`, `orch-app/src/{core,commands,lib}.rs`; contract in `api-contract.md`. `front` reviews |
| T-2 UI: outside group with Remove, copy, dialogs, Prune N gone | front | — (against `api-contract.md`, invoke is mocked in tests) | `ui/src/panes/ProjectPane.svelte`, `ui/test/`. `back` reviews |

T-1 and T-2 run in parallel; the lead merges T-1 first and checks the real pane against the user's repo before the PR.
