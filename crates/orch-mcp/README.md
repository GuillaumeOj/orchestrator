# orch-mcp

The orchestrator's MCP server (SPEC §6, ADR-009 decision 5). Claude Code runs it over **stdio** for
every agent (`mcpServers.orch` in the generated `mcp.json`); it speaks JSON-RPC 2.0, one message
per line, and forwards each tool call to the core over the bus
([orch-protocol](../orch-protocol/README.md) `{"type":"mcp"}`). The agent's identity is the
environment the core set at spawn (`ORCH_AGENT_ID`, `ORCH_SOCK`), repeated in the server's `env`.
There is no network port and no token on the command line.

## Tools

An agent sees the tools its role allows: `ORCH_TOOLS` (comma-separated, set by the core in the
server's `env` and the agent's environment); unset = a solo session's `commit`, `request_pr`. A tool
outside that list is unknown (−32602), and the core checks again (`Runtime::on_mcp`).

| Tool (`mcp__orch__…`) | Who | What the core does |
|---|---|---|
| `commit(message, paths?)` | everyone | Stages `paths` (or every change, `git add -A`) and commits in **that agent's worktree**, outside the sandbox, with hooks off and the agent's identity ([orch-git `commit`](../orch-git/README.md)). Concludes a merge left in progress. Logged in the audit log. |
| `rebase_base(abort?)` | lead | The core fetches `origin/<base>` and rebases the integration branch onto it (task merges kept; refused while a task holds unmerged work or a PR / update request waits). A conflict stops it: `commit` continues it, `abort` restores the branch. |
| `request_pr(title, body?, draft?)` | solo, lead | Creates an Inbox **gate** (push + PR). Returns at once; on approval the core pushes with the user's credentials, opens the PR with `gh`, and tells the agent the URL. Agents never push. |
| `send_message(to, body)` | team | To a node or one agent, if the team rules allow it (FR-304). |
| `inbox(all?)` | team | The agent's messages; marks them read. |
| `list_tasks(mine?, status?)` | team | The feature's tasks. |
| `handoff(artifact, to, kind?, note?)` | team | Records an Artifact on the feature and tells the recipient. |
| `request_review(to, branch?, note?)` | team | A review request (rules: `reviews` edges, the lead). |
| `ask_user(question, options?)` | team | An Inbox question; the reply comes back as a turn. |
| `create_task(title, node, body?, depends_on?, status?)` | lead | A task in the project's backend (Local / GitHub); only once the spec is approved. |
| `update_task(id, status?, notes?, node?)` | team | Status, note; reassign (lead). Specialists: own tasks, not Done. |
| `request_spec_approval(summary)` | lead | The spec gate (docs committed in `.orchestrator/features/<slug>/`). |
| `merge(branch, task?)` | lead | The core merges `orch/<slug>/<node>-<n>` into the integration branch (outside the sandbox, hooks off); a conflict aborts and the owner is told; `task` → Done. |
| `report_done(summary, task?)` | specialist | Task → Review, a report to the node it reports to. Refused with uncommitted changes. |
| `sync_integration()` | specialist | The core merges the integration branch into the agent's branch; a conflict is left for the agent to fix and commit. |

An agent's tools are listed in its generated `permissions.allow`, so the CLI does not prompt; the
core applies the store's **permission rules** before running any of them: `deny` refuses, `ask`
holds the call for an Inbox decision (up to 5 min), `allow` or no rule runs it (committing in its own
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
