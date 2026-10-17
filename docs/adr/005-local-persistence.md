# ADR-005 — Local persistence: SQLite + `.orchestrator/` for shareable docs

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (dropped strict event sourcing)

## Decision
- **App state** goes in **SQLite** in app data (WAL mode, a single writer thread, versioned migrations with refinery or similar). It holds mutable **state tables** plus an **append-only event table** for hook, statusline and governor events.
  - Streaming data is coalesced.
  - Large payloads are stored as files outside the rows.
  - Raw events are kept for 30 days by default.
  - FTS5 indexes the search.
- **Conversation history is not duplicated**: Claude Code's transcript JSONL files are the source of truth for messages. The app indexes them for search and keeps only references and metrics.
- **Shareable project docs** (feature specs, briefs, contracts, decisions) go in `<project>/.orchestrator/` on the integration branch, tracked in git. A project can opt out and keep them in app data, e.g. for client or open-source repos.
- **Secrets** go only in the macOS Keychain. Claude auth stays with Claude Code.

## Consequences
- Fast local queries for the Overview and pacing, and the app survives restarts.
- One source of truth per kind of data.
