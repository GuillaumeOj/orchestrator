# agent-runtime

The headless agent runtime of the Rust core (SPEC §4.2, §6.2–6.5, §8, J0, J9, J10). Each agent is a
real interactive `claude` process in a PTY owned by the core, configured with generated per-agent
files, observed through hooks and the statusline, and steered through hook decisions and PTY input.
No UI, no Tauri: [`orchd`](../orchd/README.md) drives it from the command line.

| Module | What it does | SPEC |
|---|---|---|
| `role`, `compile` | Role → per-agent dir (`settings.json`, `mcp.json`, `gitconfig`) + command line | §6.3, FR-702, ADR-009 |
| `env` | Environment built from scratch; PATH from the login shell; OTEL; no credentials | §6.3, FR-703, FR-1201 |
| `preflight` | `claude` version range, git ≥ 2.48, helper binaries at a stable path | NFR-7, FR-1201 |
| `pty` | `claude` in a PTY (portable-pty): own session/group, close-on-exec master, pull buffer | FR-201, FR-207, ADR-001 §4 |
| `delivery` | FR-206 input rules: idle only, `/ ! #` escape, < 900 chars or file, start delay, turn marker | FR-206, R4 |
| `state` | Agent state machine from hook events | §6.2, §6.4, §9.5 |
| `worktree` | Sibling folder, relative paths, lock, idempotent create, branch naming, inspection | FR-105, ADR-003 |
| `procs` | Process table (ps + lsof cwd), kill by argv + cwd, phys_footprint via `proc_pid_rusage` | FR-207, NFR-1b |
| `transcript` | Transcripts by session id across `~/.claude/projects/*/` (read only) | FR-209 |
| `reconcile` | Startup reconciler in J10 step 2 order | J10, NFR-2 |
| `usage` | Statusline samples merged by recency → Governor `UsageSample` | FR-801, R3 |
| `store` | Persistent state (JSON stand-in for SQLite) | ADR-005 |
| `runtime` | The core: hooks → state → Governor, PreToolUse holds, delivery pump, lifecycle | FR-202–207, FR-806 |
| `server` | Bus server on the Unix socket + control requests | NFR-3 |

## Compile (§6.3)

`compile(role, ctx)` is pure and returns the files and the command; `Compiled::write` writes them.

- **Hooks**: every event in `HookEvent::REGISTERED` (§6.3 list + PostCompact) → `<app>/bin/orch-hook`,
  matcher `*`, `timeout: 600`.
- **statusLine**: `<app>/bin/orch-status`, `refreshInterval: 5`. The user's own `statusLine.command`
  (read from `~/.claude/settings.json`) is passed in `ORCH_USER_STATUSLINE` and chained by orch-status.
- **Sandbox**: `enabled`, `failIfUnavailable`, `autoAllowBashIfSandboxed`, `allowUnsandboxedCommands: false`,
  `network.allowedDomains` from the role (package registries by default). `filesystem.denyRead`:
  credential stores (`~/.ssh`, `~/.config/gh`, `~/.netrc`, `~/.git-credentials`, `~/.aws`, gcloud, azure,
  kube, docker config, gnupg, `.npmrc`, `.pypirc`, cargo credentials, pass, Keychains; never
  `~/.gitconfig`), the main checkout **entry by entry except its `.git`**, the other worktrees, and the
  core's `state/`. `allowRead`/`allowWrite`: the git common dir.
- **Permissions**: `deny` = `Bash(git push:*)`, `Read(//…)` for each credential store, `Read`/`Edit` on the
  main checkout and the other worktrees, `Read` on `state/`, `Edit` on all of app data, plus the
  role's rules. Absolute paths always use `//`. `allow` = the role's rules.
- **Command line**: `--settings`, `--mcp-config`, `[--strict-mcp-config]`, `[--setting-sources]`,
  `[--model]`, `[--effort]`, `--permission-mode` (always), `--append-system-prompt` (ORCH preamble, then
  the role prompt), `-n <id>`, then `--session-id <uuid>` | `--resume <id>` | `--resume <id> --fork-session`.
  Never `--bare`, `--safe-mode` or `bypassPermissions` (not representable in `PermissionMode`).

## Environment

Nothing is inherited. `HOME USER LOGNAME SHELL LANG TERM=xterm-256color COLORTERM PATH TMPDIR`,
`ORCH_AGENT_ID ORCH_SOCK ORCH_AGENT_DIR ORCH_HOOK_DEADLINE_MS [ORCH_USER_STATUSLINE]`,
`DISABLE_AUTOUPDATER=1`, `GIT_CONFIG_GLOBAL=<agent dir>/gitconfig` (the user's name/email, empty
credential helper), `GIT_TERMINAL_PROMPT=0`, and with a receiver configured
`CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_*` (http/json) and `OTEL_RESOURCE_ATTRIBUTES=orch.agent_id=<id>`.
PATH comes from `$SHELL -l -i -c` with a marker (5 s deadline, fallback PATH), plus `~/.local/bin`.
`ANTHROPIC_API_KEY` is never set; a warning is raised if the app's own env has one.

## Spawn, kill, suspend

- Every spawn checks the `claude` version (range: min 2.1.302 refuses below, newer warns), the
  helpers (exist, executable, not inside an `.app`), the worktree, and recompiles the files.
- portable-pty runs `setsid` + `TIOCSCTTY`: own session and process group. The master fd gets
  `FD_CLOEXEC` explicitly.
- **Pull transport**: the reader appends to a per-agent buffer. With a consumer attached it blocks
  above 2 MB (backpressure); with none it keeps the last 256 KB (replayed on attach) and answers
  cursor/DA queries itself. `read()` drains everything buffered.
- **Kill** (FR-207): SIGTERM to `claude`'s group and to the group and pid of every **tty-less**
  process whose cwd is inside the worktree (`lsof -d cwd`), SIGKILL after 5 s. **Suspend** SIGTERMs
  `claude`'s group only (it cleans up its own shells) and records the intent; the exit callback then
  sets `suspended` (or `killed`; an exit nobody asked for is `error`).

## Delivery (FR-206)

A turn is queued, admitted by the Governor (admission point 1, only once the agent can take it),
then pasted: `ESC[200~[t:<8 hex>] <body>ESC[201~`, Enter 300 ms later. Rules: agent `idle` with no
running background task, ≥ 1.5 s after SessionStart (0.5 s after Stop), no user keystroke for 2 s,
no trust dialog. A body starting with `/`, `!` or `#` gets a leading space; tabs become spaces; a
paste over 900 chars puts the body in `<agent dir>/messages/<turn>.md` and pastes a pointer. The
marker comes back in UserPromptSubmit (→ `delivered`) and the reconciler looks for it in the
transcript. A suspended agent with queued work is resumed through the Governor first.

## State machine (§6.2, §6.4)

`SessionStart` → idle (not for `source: compact`) · `UserPromptSubmit` → working, unless its
`prompt_id` is the running turn's (a **queued message**, not a turn); origin = app (marker), user,
or self-started (`<task-notification>`, FR-410) · `PreToolUse` → working + current tool ·
`PermissionRequest` / Notification `permission_prompt` → waiting_for_you · `Stop` → idle, or
**working** when `background_tasks` has running entries · `StopFailure` → blocked_by_limit
(`rate_limit`) or error · SubagentStart/Stop tracked, internal SubagentStop (empty `agent_type`)
ignored · `SessionEnd` ignored (the process exit and the core's intent decide).

## Governor wiring

- User turns → `request_turn(interactive)`; self-started → `request_turn(self_started)`; app turns
  are requested before delivery. `Stop`/`StopFailure`/exit → `turn_ended`; StopFailure `rate_limit`
  → `rate_limit_hit` with the window that reads 100 % in the agent's last statusline, then Esc to
  cancel the CLI's auto-continue (J9).
- **PreToolUse**: `tool_call`. Admitted → no output (the CLI's own permission flow applies).
  Otherwise the hook is **held** up to `hold` (default 5 s, always 1 s inside orch-hook's deadline)
  while `tick` runs; a release for that agent allows the tool, otherwise the answer is deny
  "ORCH: paused by governor (<cause>)…". During the hold the connection is watched: EOF (Esc killed
  orch-hook) ends the hold as `interrupted`.
- PermissionRequest gets no decision (the dialog shows in the pane): there is no Inbox UI yet.
- The usage collector sends the Governor only **new responses** (readings or cost changed), with
  `ts` = receive time and the cost delta from `cost.total_cost_usd`. The first weekly reading
  re-aligns the plan week on `seven_day.resets_at` (`new_week`).
- Shadow mode (FR-812) is the default: decisions are logged with the proposed verdict.

## Reconciler (J10 step 2)

`reconcile(&ReconcileEnv)` is a dry run with `apply: false`. With `apply: true`, in order:
1. replay each agent's `spool.jsonl` through the state machine, then rename it `*.replayed.<ts>`;
2. worktrees: finish `creating` rows (add + lock + row), report `missing` (Inbox) and `unknown`;
3. merge/rebase/cherry-pick/revert in progress → **flag** (error + Inbox alert);
4. remove a stale `index.lock` when no git process has its cwd in the worktree (and after killing one);
5. kill leftovers by pgid, argv (agent dir) and cwd;
6. transcript by session id: busy states, unfinished tool uses or running background tasks →
   **resume** (a queued `ORCH: the orchestrator restarted…` turn, durable across another crash);
   else **suspend**; turns whose marker is in the transcript are `delivered`, others go back to the
   queue (and through the Governor again);
7. held gates → pending Inbox item; held governor holds → `interrupted` (the resume continues).

## Readings of the spec (simplest reading taken)

- **Persistence**: a JSON file (`state/state.json`, atomic rename) instead of SQLite. Same rows as
  §7 for what this crate needs. The Governor's own state is in memory and rebuilt at start.
- **Main checkout deny**: denying the whole checkout makes the CLI drop the `.git` grants it needs
  for worktrees (it ignores an `allowWrite` under a `denyRead`), so the checkout is denied **entry by
  entry** (top-level entries at spawn time; new ones are covered at the next spawn).
- **Kill by cwd** leaves out processes that have their own terminal and another group: the user's
  shell `cd`'d into the worktree is not the agent's.
- **Background tasks**: Stop with running `background_tasks` keeps the agent `working` (§6.2 has no
  separate state); delivery waits for the next real Stop.
- **Default plan**: weekdays 09:00–19:00 local, reserve 10 %, one allocation `default`, until the
  Week planner exists.
- **PreToolUse admitted** returns no decision rather than `allow`: an `allow` would skip the user's
  permission prompts.
- **Hold length**: "a few seconds" = 5 s by default (`--hold-secs`).

## Known gaps

- **Commits from a sandboxed worktree fail** (measured 2026-10-21, Claude Code 2.1.302): sandboxed
  Bash cannot create `<repo>/.git/worktrees/<name>/index.lock`, whatever `allowWrite` says (plain
  or `//` spelling, the whole `.git` or its subfolders), with relative or absolute worktree links,
  with no read-deny on the checkout, and with `--add-dir`. The docs say linked worktrees get this
  grant automatically. The same runs show the CLI also blocks reads outside the working directory
  by default now (`<repo>/README.md`, `<repo>/.git/worktrees/`), which P0 did not see on the same
  version: probably a server-side switch. `allowRead` on the git dir restores reads. Until this is
  resolved, agents can edit but not commit; the core (or a gated MCP tool) has to commit for them.
- No OTLP receiver yet: OTEL env is only set when an endpoint is configured.
- No orch-mcp, no Inbox decisions for PermissionRequest/gates, no Stop "continue" with the next task,
  no memory-pressure cap, no idle auto-suspend timer, no repo-move repair (J10 A3).

## Tests

- Unit tests per module, including fixture worktrees/transcripts/spools for the reconciler and the
  P0 recovery hook samples replayed through the state machine.
- `tests/e2e_fake_claude.rs`: real bus server, PTYs and helper binaries, with `tests/fake_claude.sh`
  in place of `claude` (no quota): start, delivery (marker, `/` escape), usage → Governor, second
  turn, suspend, resume (`--resume`), kill; and orch-hook with the core down.
- `tests/live_claude.rs` (opt-in, uses quota):
  `ORCH_LIVE_CLAUDE=1 cargo test -p agent-runtime --test live_claude -- --ignored --nocapture`.
  It stops after the first response if the weekly window is above 90 %. See the file header for
  `ORCH_LIVE_REPO` and `ORCH_LIVE_ANSWER_TRUST`.
