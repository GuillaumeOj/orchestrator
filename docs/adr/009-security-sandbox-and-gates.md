# ADR-009 — Security: enforced sandbox, no agent credentials, gates via hooks

- Status: Proposed · Date: 2026-10-17

## Context
The review found that permission rules alone don't enforce a "worktree sandbox":
- Claude's Read/Edit rules don't stop `cat`, `curl` or scripts run through Bash.
- Agents inherit the user's `gh` and git credentials, so they could push or open PRs and skip the PR gate.
- `canUseTool`-style prompts don't run for allow-listed tools.

## Decision
1. **OS sandbox for Bash**: each agent's generated settings enable Claude Code's sandbox (`enabled`, `failIfUnavailable`, `allowUnsandboxedCommands: false`).
   - **Writes** are limited to its worktree plus the CLI's scratch folder and tool caches. `/tmp` is blocked, and temp files go to the CLI's scratch folder.
   - **Reads are denied** (`sandbox.filesystem.denyRead`) for credential stores (`~/.ssh`, `~/.config/gh`, `~/.netrc`, `~/.git-credentials`, `~/.aws`, other cloud CLI configs) and for the main checkout and the other worktrees. The sandbox's read side is open by default, so without this, "no credentials" and "can't read other worktrees" both fail (P0).
   - **Network** is limited to a per-role allow-list (package registries, docs, configured MCP hosts).
   - Don't deny `~/.gitconfig`: it breaks git entirely (P0).
2. **Permission rules** for Claude's own tools deny the main checkout, the other worktrees, credential stores and app data. Read/Edit/Write are **not** covered by the OS sandbox, only by these rules. **Absolute paths must be written `//abs/path`**: a single `/` is relative to the settings file and silently matches nothing (P0).
3. **No credentials**:
   - The agent environment is built from scratch, with no `GH_TOKEN`/`GITHUB_TOKEN` and no `SSH_AUTH_SOCK`.
   - Git uses a generated global config (`GIT_CONFIG_GLOBAL`) with no credential helper, plus `GIT_TERMINAL_PROMPT=0`.
   - Removing env vars is not enough on its own: in P0 the SSH key on disk and the `gh` keyring token still reached GitHub until the files were made unreadable (decision 1).
   - A `Bash(git push:*)` deny rule is kept as defense in depth only, because a push inside a script bypasses it. Deny rules do win over a hook's `allow`.
   - Pushing, opening PRs and merging into protected branches is done **by the app** through gated orchestrator MCP tools, with the user's credentials kept in the core.
4. **Gates through hooks**:
   - PreToolUse runs for every tool call, including allow-listed ones.
   - PermissionRequest answers permission dialogs from the Inbox.
   - PostToolBatch records calls denied by permission rules, which reach neither PreToolUse nor PermissionDenied.
   - `orch-hook` always answers before its timeout, because timeouts fail open.
   - A gated action is denied with "awaiting user approval", so the turn ends and the agent can be suspended.
   - On approval, the app resumes the agent with "approved: proceed".
5. **Orchestrator MCP** runs over stdio. `orch-mcp` connects to the core through a Unix socket with 0600 permissions, so there is no network port and no token in the command line. The agent's identity comes from the environment the core sets. Recipients are checked against the team graph on the server side.
6. **Audit**: every gate decision, permission rule change and denied action is logged.
7. **Secrets**: copying `.env` into worktrees is opt-in, with a warning that agents send what they read to the model.

## Acceptance tests (P0)
An agent instructed to:
- write outside its worktree, to `$HOME` or to `/tmp`
- read another worktree or a credential file (Bash and the Read tool)
- `curl` an unlisted host
- run `ssh -T git@github.com` or `gh api user`
- `git push` (directly and inside a script)
- run `gh pr create`

must fail every time, and each attempt must appear in the audit log. Status: all of these pass with the hardened settings in the [P0 spike](../spikes/p0-claude-cli.md) §8.

Still untested:
- keychain access from inside the sandbox
- Docker and browsers for tests
- whether the model tries `dangerouslyDisableSandbox`

## Consequences
- The PR and merge gates can't be skipped.
- Some tools need extra sandbox allowances (e.g. Docker or browsers for tests); these are configured per role, and each one is visible in its settings.
