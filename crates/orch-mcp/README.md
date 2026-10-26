# orch-mcp

The orchestrator's MCP server (SPEC §6, ADR-009 decision 5). Claude Code runs it over **stdio** for
every agent (`mcpServers.orch` in the generated `mcp.json`); it speaks JSON-RPC 2.0, one message
per line, and forwards each tool call to the core over the bus
([orch-protocol](../orch-protocol/README.md) `{"type":"mcp"}`). The agent's identity is the
environment the core set at spawn (`ORCH_AGENT_ID`, `ORCH_SOCK`), repeated in the server's `env`.
There is no network port and no token on the command line.

## Tools (P1)

| Tool (`mcp__orch__…`) | What the core does |
|---|---|
| `commit(message, paths?)` | Stages `paths` (or every change, `git add -A`) and commits in **that agent's worktree**, outside the sandbox, with hooks off and the agent's identity ([orch-git `commit`](../orch-git/README.md)). Logged in the audit log. |
| `request_pr(title, body?, draft?)` | Creates an Inbox **gate** (push + PR). Returns at once; on approval the core pushes with the user's credentials, opens the PR with `gh`, and tells the agent the URL. Agents never push. |

Both are listed in the generated `permissions.allow`, so the CLI does not prompt; the core applies
the store's **permission rules** before running them: `deny` refuses, `ask` holds the call for an
Inbox decision (up to 5 min), `allow` or no rule runs it (the P1 default: committing in its own
worktree is the agent's normal work, FR-701).

## Behaviour

- `initialize` answers with the client's protocol version when it is one of `2025-06-18`,
  `2025-03-26`, `2024-11-05` (else the newest), capability `tools`.
- `tools/list`, `tools/call`, `ping`; empty `resources/list` and `prompts/list`; notifications get no
  reply; batches are answered as batches; unknown methods −32601, unknown tools −32602.
- **Core unreachable** (missing or stale socket, timeout, bad reply): a tool **error** for the model
  ("orchestrator unavailable … Nothing was committed or requested"), never a crash or a hang. The
  wait for an answer is 300 s (`DEFAULT_MCP_DEADLINE_MS`).

Tests: `cargo test -p orch-mcp` (handshake, list, unknowns, core down, forwarding to a fake core),
plus the agent-runtime e2e test, where the fake claude commits through the real `orch-mcp` binary.
