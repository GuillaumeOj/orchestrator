# ADR-003 — Isolation: one git worktree per agent + integration branch per feature

- Status: Proposed · Date: 2026-10-17

## Context
Many agents edit the same repository in parallel. Sharing a checkout leads to clobbered files and broken builds. The user wants worktrees to stay **inside the repository folder** so everything about a project lives in one place.

## Decision
- Each agent/session gets its own **git worktree** and branch: `orchestrator/<feature>/<role>-<n>`.
- Worktrees live **inside the repo** at `<repo>/.orchestrator/worktrees/<feature>/<role>-<n>/`.
- The `worktrees/` folder is ignored through **`.git/info/exclude`**, so the app never has to modify the project's tracked `.gitignore`. The rest of `.orchestrator/` (feature specs, etc.) stays tracked.
- Each feature has an **integration branch** `orchestrator/<feature>`. Specialist branches are created from it and the Lead merges them back.
- A per-project **setup script** runs on worktree creation (install deps, copy `.env`).
- The PR to the default branch is opened from the integration branch, behind the **PR gate**.
- The location can be changed per project (e.g. to a sibling folder) if a project's tooling misbehaves.

## Side effects of nesting worktrees in the repo (none blocking)
| Effect | Mitigation |
|---|---|
| Tools run from the main checkout (ripgrep, tsc, eslint, jest/vitest, bundlers, file watchers, IDE indexers) may scan the worktrees and see duplicate sources. | Most tools respect git ignores (ripgrep, fd). For the rest, the app offers to add `.orchestrator/worktrees` to the tool's excludes (tsconfig `exclude`, test `testPathIgnorePatterns`…) and shows a warning in project settings. |
| Node/Python module resolution walks up parent folders, so a worktree may resolve a package from the main checkout's `node_modules`. | Each worktree runs its own install (setup script). Prefer pnpm with a shared store (fast, little extra disk). |
| Agents running in the main checkout could read or edit files in other agents' worktrees. | Permission rules deny access to `.orchestrator/worktrees/**` except the agent's own worktree. Out-of-sandbox access is a gate. |

## Consequences
- Safe parallelism, a clear history per agent, and everything for a project kept in its folder.
- Disk usage and install time per worktree, mitigated by pnpm/shared caches and cleanup tooling (FR-104).
- Conflicts surface at integration time and are routed to the owner agent.
