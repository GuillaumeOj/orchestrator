# ADR-005 — Local persistence: SQLite + `.orchestrator/` for shareable docs

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-26 (P1 core services: `crates/orch-store`)

## Decision
- **App state** goes in **SQLite** in app data (WAL mode, a single writer thread, versioned migrations with refinery or similar). It holds mutable **state tables** plus an **append-only event table** for hook, statusline and governor events.
  - Streaming data is coalesced.
  - Large payloads are stored as files outside the rows.
  - Raw events are kept for 30 days by default; forecast snapshots 30 days, usage samples 90 days, decided Inbox items 90 days. The audit log is never purged.
  - FTS5 indexes the search.
  - Migrations are a numbered list in code, applied in order and recorded in `PRAGMA user_version` (no external migration tool); a database newer than the build is refused.
  - `synchronous=NORMAL` in WAL: a crash of the process never loses a committed transaction (tested with SIGKILL); a power loss can lose the last ones.
  - The agent runtime keeps its own tables (agents, worktrees, turns, holds, Inbox) in memory for its hot paths and writes every change through, one transaction per change. Other code changes those rows through the runtime only.
- **Conversation history is not duplicated**: Claude Code's transcript JSONL files are the source of truth for messages. The app indexes them for search (user and assistant text and tool calls, incrementally from the last indexed byte) and keeps only references and metrics. Transcripts are found **by session id** across `~/.claude/projects/*/`, because a resumed session keeps writing to the transcript in its original project folder after a repo move.
- **Shareable project docs** (feature specs, briefs, contracts, decisions) go in `<project>/.orchestrator/` on the integration branch, tracked in git. A project can opt out and keep them in app data, e.g. for client or open-source repos.
- **Secrets** go only in the macOS Keychain. Claude auth stays with Claude Code.

## Consequences
- Fast local queries for the Overview and pacing, and the app survives restarts.
- One source of truth per kind of data.
