# ADR-005 — Local persistence: SQLite + per-project `.orchestrator/`

- Status: Proposed · Date: 2026-10-17

## Decision
- **App state** (projects, sessions, events, usage, plans, governor log, teams/roles) is stored in **SQLite** in the app data directory, with versioned migrations. Session events are append-only.
- **Shareable project artifacts** (feature specs, UX briefs, API contracts, decisions, optional task mirror) go in **`<project>/.orchestrator/`**, tracked in git and editable in nvim.
- Secrets go only in the macOS Keychain. Claude auth stays with Claude Code.

## Consequences
- Fast local queries for monitoring and pacing; resilient to restarts.
- Specs travel with the code and are reviewable in PRs.
