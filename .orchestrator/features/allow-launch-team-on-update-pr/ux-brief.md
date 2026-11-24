# Allow to Launch a Team on Update PRs · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). The design file is the FigCraft file **in
> Google Drive** (the project file `Orchestrator.figcraft`). Do **not** commit `docs/design/Orchestrator.figcraft`.
> Start from: the Project pane page (Pull requests card), page **04.4** (Feature, Launch dialog, P4.5 prefilled
> dialogs and the Origin chip), the gate card components, and the delete-feature dialog.
> Notes: `docs/design/allow-launch-team-on-update-pr-design-notes.md`. Exports under
> `.orchestrator/features/allow-launch-team-on-update-pr/design/` (one PNG per board, prefix `pr-`, 1512 and 1280).

## 1. Principles
- **Someone else's PR**: everywhere the team will touch it, say so plainly (whose PR, which branch, that the push
  goes to it). Never suggest a new PR is opened.
- **Starting stays yours**: one dialog the user submits, prefilled, editable.
- **Reuse**: the launch dialog, origin chip, gate card and PR card exist; this feature adds variants, not new
  surfaces. No new token.

## 2. ux — notes first, then hand off to ui
Settle each with copy, states and keys; add a "Findings for lead" table (contract gaps, F1…).
1. **PR list row** (Project › Pull requests): anatomy (number, title, author with a bot mark, head → base, CI, review,
   draft); the action per state: Launch team… · Open feature (D3, with the run status) · disabled with reason (fork);
   key (the pane already uses `p` for Load PRs: propose one for the focused row, or none); loading / gh error / empty.
2. **Launch dialog, PR variant**: "From PR #42" origin chip; **Works on** line (PR link, `head → base`, author)
   replacing Base and the PR mode control; brief prefilled (failing checks excerpts: how long is it allowed to look
   before scrolling); copy "Checks out renovate/foo" in place of "Creates orch/…/integration"; prefill loading state
   (gh calls take seconds) and its errors (closed, merged, fork, offline); launch errors inline (branch checked out
   elsewhere with its path; local branch ahead).
3. **Feature pane** of a linked feature: where "PR #42 · renovate/foo → main · opened by renovate[bot]" lives
   (header and/or PR card) from launch, before any push; the **head moved outside the app** warning; closed PR state.
4. **Gate card** for the push: "Push 3 commits to PR #42 (renovate/foo)", what it lists (only the team's commits),
   approve label, stale (moved head) outcome.
5. **Delete feature** dialog for a linked feature: the real branch; "kept: not pushed to PR #42" rule (D2); nothing
   deleted on GitHub.
6. **States**: PR list with a linked active feature, a fork PR, a draft PR; prefill loading / failed; linked feature
   before push, after push (pr open), head moved, PR closed, PR merged.

## 3. ui — FigCraft, after ux hands off
Boards (1512 and 1280 where the layout changes): `pr-list` (row states), `pr-launch-dialog` (prefilled, loading,
error), `pr-feature-linked` (before push, head moved, closed), `pr-gate-push`, `pr-delete-feature`. Export each PNG to
the `design/` folder above, add the rows to `docs/design/README.md`, commit the exports and notes.

## 4. Reviews
- **ui** reviews F1's screens against the exports (must fix / acceptable).
- **ux** checks copy and states in the built UI and reports to lead. ux does not change code.
