# ADR-002 — Agent runtime: Claude Agent SDK behind a Provider adapter

- Status: Proposed · Date: 2026-10-17

## Context
Claude Code is the primary tool, but we want to add other agent CLIs later (Codex, Gemini…) without rewriting the app.

## Decision
- Drive Claude through the **Claude Agent SDK**, which reuses the user's Claude Code login and plan.
- Define a **`Provider` interface** (SPEC §8) with capability flags. The core only talks to the interface.
- Normalize all provider output into an internal **event stream** (message, tool_call, tool_result, usage, rate_limit, context, error, done).
- Roles/Teams are **compiled** per provider into agent definitions (prompt, tools, MCP servers, permissions).

## Consequences
- The UI and Governor are provider-agnostic; unsupported features are hidden through capability flags.
- Adapters for CLI-only tools may need to parse stdout/JSON streams, so their fidelity will vary.
