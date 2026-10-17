# ADR-009 — Security: enforced sandbox, no agent credentials, gates via hooks

- Status: Proposed · Date: 2026-10-17

## Context
The review found that permission rules alone don't enforce a "worktree sandbox":
- Claude's Read/Edit rules don't stop `cat`, `curl` or scripts run through Bash.
- Agents inherit the user's `gh` and git credentials, so they could push or open PRs and skip the PR gate.
- `canUseTool`-style prompts don't run for allow-listed tools.

## Decision
1. **OS sandbox for Bash**: each agent's generated settings enable Claude Code's sandbox, with writes limited to its worktree (plus temp and tool caches) and network limited to a per-role allow-list (package registries, docs, configured MCP hosts).
2. **Permission rules** for Claude's own tools deny the main checkout, the other worktrees and app data.
3. **No credentials**: the agent environment has no `GH_TOKEN`/`GITHUB_TOKEN`, no git credential helper, no `SSH_AUTH_SOCK`; `git push` is denied by rule as well.
   - Pushing, opening PRs and merging into protected branches is done **by the app** through gated orchestrator MCP tools, with the user's credentials kept in the core.
4. **Gates through PreToolUse hooks** (they run for every tool call, including allow-listed ones).
   - A gated action is denied with "awaiting user approval", so the turn ends and the agent can be suspended.
   - On approval, the app resumes the agent with "approved: proceed".
5. **Orchestrator MCP** runs over stdio. `orch-mcp` connects to the core through a Unix socket with 0600 permissions, so there is no network port and no token in the command line. The agent's identity comes from the environment the core sets. Recipients are checked against the team graph on the server side.
6. **Audit**: every gate decision, permission rule change and denied action is logged.
7. **Secrets**: copying `.env` into worktrees is opt-in, with a warning that agents send what they read to the model.

## Acceptance tests (P0)
An agent instructed to:
- write outside its worktree
- read another worktree
- `curl` an unlisted host
- `git push`
- run `gh pr create`

must fail every time, and each attempt must appear in the audit log.

## Consequences
- The PR and merge gates can't be skipped.
- Some tools need extra sandbox allowances (e.g. Docker or browsers for tests); these are configured per role, and each one is visible in its settings.
