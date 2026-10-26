# ADR-009 — Security: enforced sandbox, no agent credentials, gates via hooks

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-26 (P1 core services: commits through `orch-mcp`, PR gate, rule evaluation)

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
   - The main checkout is denied **entry by entry, except its `.git`**, and the repository's git dir gets `allowRead` and `allowWrite`: the worktree's git needs it, and the CLI drops an `allowWrite` that lies under a `denyRead` path. Top-level entries created later are covered at the next spawn.
   - Sandbox paths are plain absolute paths (`/x`, `~/x`); the `//` form is for permission rules only.
   - **Known gap (2026-10-21, Claude Code 2.1.302)**: sandboxed Bash in a linked worktree still cannot write the repository's git dir (`index.lock` denied), whatever `allowWrite` or `--add-dir` says, although the docs say linked worktrees get this grant. Agents can't `git commit`; they call the orchestrator MCP tool `commit` and the core commits for them (decision 5, SPEC FR-702, R10). The same runs showed the sandbox blocking reads outside the working directory by default, unlike P0.
2. **Permission rules** for Claude's own tools deny the main checkout, the other worktrees, credential stores and app data. Read/Edit/Write are **not** covered by the OS sandbox, only by these rules. **Absolute paths must be written `//abs/path`**: a single `/` is relative to the settings file and silently matches nothing (P0).
3. **No credentials**:
   - The agent environment is built from scratch, with no `GH_TOKEN`/`GITHUB_TOKEN` and no `SSH_AUTH_SOCK`.
   - Git uses a generated global config (`GIT_CONFIG_GLOBAL`) with no credential helper, plus `GIT_TERMINAL_PROMPT=0`.
   - Removing env vars is not enough on its own: in P0 the SSH key on disk and the `gh` keyring token still reached GitHub until the files were made unreadable (decision 1).
   - A `Bash(git push:*)` deny rule is kept as defense in depth only, because a push inside a script bypasses it. Deny rules do win over a hook's `allow`.
   - Pushing, opening PRs and merging into protected branches is done **by the app** through gated orchestrator MCP tools, with the user's credentials kept in the core.
   - Killing an agent (SPEC FR-207) targets `claude`'s process group and the processes **with no controlling terminal** whose cwd is in its worktree; the user's own shells there are left alone.
4. **Gates through hooks**:
   - PreToolUse runs for every tool call, including allow-listed ones.
   - PermissionRequest answers permission dialogs from the Inbox.
   - PostToolBatch records calls denied by permission rules, which reach neither PreToolUse nor PermissionDenied.
   - `orch-hook` always answers before its timeout, because timeouts fail open.
   - **Fail closed when the core is down** (P0 recovery spike): if the socket is missing, refused, closes mid-hold or misses the deadline, PreToolUse and PermissionRequest answer deny; other events exit 0 and are spooled for replay. A **missing hook binary fails open**, so the helpers live at a stable path in app data and are checked at every spawn.
   - A gated action is denied with "awaiting user approval", so the turn ends and the agent can be suspended.
   - On approval, the app resumes the agent with "approved: proceed".
5. **Orchestrator MCP** runs over stdio. `orch-mcp` connects to the core through a Unix socket with 0600 permissions (path under macOS's 104-byte limit, stale file removed at start), so there is no network port and no token in the command line. The agent's identity comes from the environment the core sets. Recipients are checked against the team graph on the server side.
   - P1 tools: `commit(message, paths?)` and `request_pr(title, body?, draft?)`. Both are allowed in the generated settings (no CLI prompt); the core evaluates the store's permission rules before running them (deny > ask > allow, scopes project / role / agent): *deny* refuses, *ask* holds the call for an Inbox decision, otherwise it runs.
   - **Commits run outside the sandbox**, so nothing the agent wrote may execute: only in that agent's linked worktree (never a main checkout, never while `index.lock` exists), git hooks off (`core.hooksPath=/dev/null`, `--no-verify`), no signing, a cleared environment with the agent's generated `GIT_CONFIG_GLOBAL`, literal pathspecs that must stay inside the worktree. Running repo hooks inside the agent's sandbox is a possible later option.
   - `request_pr` creates an Inbox gate; on approval the app pushes the branch with the user's credentials and opens the PR with `gh`, then tells the agent. `orch-mcp` answers a tool error ("orchestrator unavailable", nothing done) when the core is unreachable.
6. **Audit**: every gate decision, permission rule change, denied action, agent commit and PR is logged in an audit table that retention never purges (SPEC FR-709).
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
