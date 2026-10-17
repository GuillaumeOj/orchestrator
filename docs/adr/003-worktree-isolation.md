# ADR-003 — Isolation: one worktree per agent, in a sibling folder

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (moved from inside the repo to a sibling folder)

## Context
Many agents edit the same repository in parallel, so each one needs its own checkout. Nesting worktrees inside the repo had real costs:
- Claude Code loads CLAUDE.md files from parent folders, so agents would read the main checkout's CLAUDE.md.
- Indexers, Spotlight, Time Machine, file watchers and Docker contexts would crawl every copy.
- Tracked and ignored content would be mixed in `.orchestrator/`.

## Decision
- Worktrees live in a **sibling folder**: `<parent>/<repo>.worktrees/<feature>/<role>-<n>/` and `<parent>/<repo>.worktrees/solo/<slug>/`. The root can be changed per project.
- **Branch names** (no ref can be both a file and a folder):
  - `orch/solo/<slug>` for solo sessions
  - `orch/<feature>/integration` for a feature's integration branch
  - `orch/<feature>/<role>-<n>` for team agents
- The **Lead owns the integration worktree** (one branch can't be checked out in two worktrees) and does all merges. Specialists branch from the integration branch.
- Feature docs (`.orchestrator/features/<slug>/`) have one source of truth: **the copy on the integration branch**.
- Worktrees are `git worktree lock`ed while in use. The startup reconciler runs `git worktree list --porcelain` and `git worktree repair` when the repo has moved.
- **Workspace trust**: the user trusts `<repo>.worktrees/` once (a one-time pane at project setup), and every worktree under it inherits the trust. Otherwise Claude Code's trust dialog would block each new agent before any hook runs. To confirm in a real worktree root (SPEC R10).
- A per-project setup script runs on creation. Copying secrets (`.env`) is opt-in, with a warning.
- The PR to the default branch is opened by the app from the integration branch, behind the PR gate (agents have no push credentials, ADR-009).

## Consequences
- Clean isolation; tools working in the main checkout don't see agent copies.
- Disk space and install time per worktree; pnpm/shared caches and cleanup tooling mitigate it.
- The sibling folder must be created next to the repo, so the user needs write access to the parent folder (otherwise the per-project root is used).
