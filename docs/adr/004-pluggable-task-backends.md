# ADR-004 — Pluggable task backends (Local / GitHub / Linear)

- Status: Proposed · Date: 2026-10-17

## Context
Some projects are tracked in Linear, others in GitHub, and some don't need a tracker. The Lead agent still needs a task list to drive specialists.

## Decision
- A `TaskBackend` interface: `list`, `create`, `update`, `link`, `subscribe`.
- Three implementations: **Local** (SQLite, optional `.orchestrator/tasks.json` mirror), **GitHub Issues**, **Linear**. Selected per project.
- Agents never call the backend directly. They use the internal MCP tools (`create_task`, `update_task`…), which route to the configured backend.
- The UI board renders a uniform model on top of any backend.

## Consequences
- Agents' prompts and protocol are identical whichever backend is used.
- Two-way sync requires conflict rules (remote wins + notification).
