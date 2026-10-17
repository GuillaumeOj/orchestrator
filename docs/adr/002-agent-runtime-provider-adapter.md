# ADR-002 — Agent runtime: the official Claude Code CLI, observed and steered

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (supersedes "Agent SDK behind a Provider adapter")

## Context
- The user wants to use their **Claude Max plan**. Anthropic's support docs say API-key usage is "billed at standard API rates (distinct from Pro/Max Plan pricing)", and that monthly API credits included with Max can't be used in Claude Code. Plan usage only works through the official Claude Code client.
- The Agent SDK docs say: "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK."
- The 5h/weekly utilization % and reset times are documented in the **CLI statusline data** (`rate_limits.five_hour|seven_day.used_percentage`, `resets_at`, Pro/Max only), not in the SDK.
- Rebuilding Claude Code's chat UI would be the most expensive and least differentiating part of the app.

## Decision
- Every agent is a **real interactive `claude` process in a PTY pane**, logged in with the user's subscription. The app never sets `ANTHROPIC_API_KEY`, and warns if one is present.
- The app **configures** each agent through generated per-agent files (settings with hooks, statusline, sandbox and permissions; MCP config; flags; environment). It never touches the user's `~/.claude`.
- It **observes** each agent through hooks (`orch-hook`), statusline JSON (`orch-status`), OpenTelemetry metrics (local receiver) and transcript JSONL.
- It **steers** each agent through hook decisions (PreToolUse allow/deny, PostToolUse `additionalContext`, Stop "continue"), the orchestrator MCP server (`orch-mcp`, stdio, one per agent, identity from the environment) and PTY input while the agent is idle.
- A small `Provider` trait (SPEC §8) isolates these details. **Claude Code CLI is the only implementation**; other CLIs are a "could".

## Alternatives
- **Agent SDK + API key**: documented, but billed per token and none of the Max allowance is used. Rejected by the user.
- **Agent SDK + subscription login**: conflicts with the SDK docs note. Rejected.
- **Headless `claude -p` stream-json**: structured output, but loses the interactive UI and the statusline signals. It may be used later for short internal jobs.

## Consequences
- The chat is Claude Code itself, so feature parity comes for free.
- Control is coarser than an SDK. The Governor admits at turn start and at each tool call (ADR-007).
- Hooks, the statusline format and flags are not a stable API: the app declares a supported Claude Code version range and has contract tests (SPEC R2, NFR-7).
- Heavy unattended automation on a subscription needs confirmation from Anthropic (SPEC R1, P0 go/no-go).
