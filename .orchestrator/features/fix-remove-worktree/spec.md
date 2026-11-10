# Fix Remove Worktree From Project's Overview · Feature spec

> Feature `fix-remove-worktree` · Integration branch `orch/fix-remove-worktree/integration` · Base `main` @ c13b52b · back + front, no design pass (no new UI, copy only)

## 1. Problem

In the project pane (the project's overview, `ui/src/panes/ProjectPane.svelte`), the user clicks **Remove** on a worktree of the app's sibling folder and confirms. Whether the worktree's folder still exists on disk or not, the worktree is **still listed** afterwards.

**Goal:** once a removal succeeds, the worktree leaves the list at once and does not come back. When a removal is refused, the user sees why and the row stays.

## 2. What happens today (HEAD c13b52b)

1. `removeWorktree(w)` → confirm → `act('wt', …)`: `invoke('worktree_remove', { project, path, deleteBranch: true, discard })`, a toast, then `act`'s `finally` calls `load()` **directly** (and `refresh()`).
2. `load()` calls `project_detail` and assigns `d`. The list (`managed`) is derived from `d.worktrees`.
3. The same pane also calls `load()` from `onTopics(['project:<id>'], load)`. `project:<id>` fires on every project upsert (fetch) and every feature/team write (`orch-store/src/teams.rs:937`), so it fires often while a team runs.
4. Core: `Runtime::remove_worktree` (`agent-runtime/src/projects.rs:139`) → `cleanup::remove_worktree` (`orch-git/src/cleanup.rs:231`) runs git (`worktree remove`, or `prune` when the folder is gone; unlock first), deletes the branch, then marks the store row `Removed`. `project_detail` → `cleanup::worktrees` lists what git knows plus the live (not `Removed`) rows.

### Findings

- **The core removes correctly.** A throwaway runtime test (not committed) covered a team worktree with its folder present, its leaf folder deleted, its whole feature folder deleted, and a solo worktree already pruned from git. In every case `remove_worktree` returned OK and the next `project_detail` no longer listed it. The existing tests never check the listing after a removal.
- **F1 · Stale reload overwrites the fresh list (likely root cause).** The pane has two loaders that don't know about each other: `act` calls `load()` directly, outside `onTopics`'s single-flight. `project_detail` is slow on a real project (status, ahead/behind and `inspect` for each of dozens of worktrees). A topic-triggered `load()` that read git **before** the removal can resolve **after** `act`'s load and put the old list back. No later topic is guaranteed, so the stale row stays until something else changes the project or the pane is reopened. `loadPrs` also assigns `d` with the same race.
- **F2 · The removal doesn't announce itself.** `remove_worktree` doesn't touch `project:<id>`. Other open views of the project (a second pane, the window after a focus change) only update by chance.
- **F3 · Exact-key row update.** The row is marked `Removed` with `s.worktrees.get_mut(&wt)`, an exact path key, while the listing matches rows by `real(path)`. A row stored under a non-canonical spelling (an older row, a relocated project, case on macOS) stays live and comes back as `missing` once git forgot the worktree.
- **F4 · `missing` rows keep their branch.** When git no longer knows the worktree (verdict `missing`), the branch comes from git's entry only, so the row's `orch/…` branch is never deleted and stays in the "orch/ branches" line.
- **F5 · Refusals look like nothing happened.** A worktree used by an agent that is neither archived nor killed is refused (`agent X uses this worktree: archive or kill it first`). The only signal is a toast. The row shows the agent id, but **Remove** looks enabled and clickable.

The fix owners first confirm F1 on the real app if they can (a log or a delayed `project_detail` in a test). They fix F1–F5 in any case: each one is a real defect.

## 3. Scope

### Core (`back`)

- **B1 (F2):** after a successful `remove_worktree`, touch `project:<project id>` so every open view reloads.
- **B2 (F3):** mark the row `Removed` by canonical path: find the row whose `real(path)` equals the canonical `wt`, not only the exact key. `remove_project`'s row updates (`orch-git/src/project.rs:206`) get the same treatment if they use the exact key too.
- **B3 (F4):** when git doesn't know the worktree, fall back to the row's branch for `delete_branch`. The usual rules still apply (`-d` when safe, `-D` only with discard, never the base or a checked-out branch).
- **B4 (F5, data only):** `project_detail`'s worktree entries carry `in_use_by: [agent ids]` (agents neither archived nor killed whose worktree is this one, the same rule as `worktree_users`), so the UI can explain a refusal before the click.
- **B5:** removing a worktree that is already gone from git and whose row is already `Removed` (a double click, two panes) returns OK with `worktree_removed: false` and changes nothing. It doesn't fail.
- Tests (`agent-runtime/src/projects.rs` and `orch-git/src/cleanup.rs`): after `remove_worktree`, `project_detail` no longer lists the worktree in each case (folder present, leaf folder gone, feature folder gone, unknown to git with a row, no row at all); the topic is touched; a non-canonical row key is still marked `Removed`; a `missing` row's branch is deleted; `in_use_by` is filled; a second removal is a no-op.

### UI (`front`)

- **U1 (F1):** one loader per pane. `act`'s reload, `loadPrs` and the topic loads all go through the same serialized loader (the `run` that `onTopics` returns, or a request counter) so a response older than the latest request is dropped, never assigned to `d`.
- **U2:** after `worktree_remove` succeeds, drop that path from `d.worktrees` right away (no flash of the old row while the reload runs). The reload then confirms it.
- **U3 (F5):** a worktree with `in_use_by` non-empty: **Remove** is disabled, and its `title` (and the state cell) says `used by <ids>: archive or kill it first`. A refused removal (an error from the core) still shows the core's error toast and leaves the row.
- Tests in `ui/test/` (vitest, the pane's existing test file or a new `projectpane.test.js`): a stale `project_detail` response that resolves after a fresh one doesn't bring a removed row back; the row disappears right after a successful removal; **Remove** is disabled with the reason when `in_use_by` is set.

Out of scope: the J11 project removal flow (except B2's row lookup), branch-only cleanup, the user's own worktrees outside the sibling folder (still listed, never removed), the agent delete dialog.

## 4. Acceptance

1. In the project pane, removing a worktree (folder present, folder deleted by hand, whole feature folder deleted, or already pruned from git) makes it leave the list immediately. It's still gone after the next topic reload, a fetch and reopening the pane.
2. While a team runs (frequent `project:` topics), a removal isn't undone by a reload that started before it.
3. A worktree in use by a live agent can't be removed from the pane, and the pane says which agent holds it.
4. A `missing` worktree's `orch/` branch is deleted with it (when merged, or with discard).
5. `cargo test` (with `ORCH_TEST_TMP`), `cargo clippy`, `bun run test`, `bun run check` and `bun run lint` pass.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-1 Core: removal announces itself, canonical row lookup, `missing` branch, `in_use_by`, idempotent | back | — | `agent-runtime/src/projects.rs`, `orch-git/src/cleanup.rs`, `orch-git/src/project.rs`; contract in `api-contract.md` |
| T-2 UI: single loader, immediate removal, disabled Remove with reason | front | T-1 (for `in_use_by`; U1/U2 can start at once) | `ui/src/panes/ProjectPane.svelte`, `ui/test/` |

`front` reviews T-1 and `back` reviews T-2.
