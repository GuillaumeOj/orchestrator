# ADR-004 — Pluggable task backends (Local / GitHub / Linear)

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (no tracked tasks.json mirror)

## Context
Some projects are tracked in Linear, others in GitHub, some in neither. The Lead still needs a task list to drive specialists.

## Decision
- A `TaskBackend` interface: `list`, `create`, `update`, `link`, `subscribe`.
- Implementations: **Local** (app SQLite), **GitHub Issues** (P2), **Linear** (P3), chosen per project.
- Tasks are assigned to **team nodes**, not to sessions (sessions come and go).
- Agents never call a backend directly; they use the orchestrator MCP tools (`create_task`, `update_task`…), which route to the configured backend.
- The UI board renders one model over every backend.
- There is no tracked `tasks.json` mirror: it would conflict across branches.

## Consequences
- Agent prompts and protocol are the same whichever backend is used.
- Two-way sync needs conflict rules: the remote wins and the user is notified.
