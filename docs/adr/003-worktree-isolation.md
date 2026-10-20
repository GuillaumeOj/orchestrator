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
- Worktrees are created with **`worktree.useRelativePaths=true`** (git ≥ 2.48; it sets `extensions.relativeWorktrees`, so older git can't open the repo), so moving the repo together with its sibling folder needs no repair. They are `git worktree lock`ed while in use; locks survive crashes and moves and keep worktrees out of `prune`. If the links break anyway, the reconciler runs `git -C <repo> worktree repair <every known worktree path>` (a plain `repair` from the main checkout can't find moved worktrees) and re-locks.
- **Workspace trust** is keyed by the **main repository root** (P0 recovery spike): once the user trusts the project root, its worktrees are trusted wherever they live. Trusting `<repo>.worktrees/` itself does *not* help. After a repo move the trust dialog comes back once.
- A per-project setup script runs on creation. Copying secrets (`.env`) is opt-in, with a warning.
- The PR to the default branch is opened by the app from the integration branch, behind the PR gate (agents have no push credentials, ADR-009).

## Consequences
- Clean isolation; tools working in the main checkout don't see agent copies.
- Disk space and install time per worktree; pnpm/shared caches and cleanup tooling mitigate it.
- The sibling folder must be created next to the repo, so the user needs write access to the parent folder (otherwise the per-project root is used).
