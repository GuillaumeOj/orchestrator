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
| `worktree` | Re-export of [`orch_git::worktree`](../orch-git/README.md) (moved there): sibling folder next to the **main checkout** (also when the project folder is a linked worktree), relative paths, lock, idempotent create | FR-105, ADR-003 |
| `procs` | Process table (ps + lsof cwd), kill by argv + cwd, phys_footprint via `proc_pid_rusage` | FR-207, NFR-1b |
| `transcript` | Transcripts by session id across `~/.claude/projects/*/` (read only) | FR-209 |
| `reconcile` | Startup reconciler in J10 step 2 order | J10, NFR-2 |
| `usage` | Statusline samples merged by recency → Governor `UsageSample` | FR-801, R3 |
| `store` | The runtime's rows in memory, written through to SQLite ([orch-store](../orch-store/README.md)) per change | ADR-005, NFR-2 |
| `runtime` | The core: hooks → state → Governor, PreToolUse holds, delivery pump, lifecycle, housekeeping | FR-202–207, FR-806 |
| `inbox` | PermissionRequest from rules or the Inbox, decisions, always-allow rules, PR gate | FR-705, FR-707–709 |
| `permissions` | Rule patterns and evaluation (deny > ask > allow, project/role/agent scopes) | FR-705, FR-708 |
| `mcp` | orch-mcp tool calls: `commit` and `request_pr`, under the rules, audited | FR-702 gap, ADR-009 |
| `resources` | Memory-pressure live cap and idle auto-suspend (pure policy + probe) | FR-203, FR-204 |
| `otlp` | Local OTLP/HTTP JSON receiver (`api_request` events → usage samples) | FR-801, NFR-4 |
| `projects` | Projects and git through the runtime (register, status, diff, cleanup, J11, relocate) | FR-101–111 |
| `server` | Bus server on the Unix socket + control requests, loops (pump, tick, housekeeping, fetch) | NFR-3 |

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
- **MCP**: `mcpServers.orch` = `<app>/bin/orch-mcp` (stdio, `ORCH_AGENT_ID`/`ORCH_SOCK` in its env),
  always present, on top of the role's servers. The preamble tells the model to commit with
  `mcp__orch__commit` and ask for PRs with `mcp__orch__request_pr`.
- **Permissions**: `deny` = `Bash(git push:*)`, `Read(//…)` for each credential store, `Read`/`Edit` on the
  main checkout and the other worktrees, `Read` on `state/`, `Edit` on all of app data, plus the
  role's rules. Absolute paths always use `//`. `allow` = the role's rules + `mcp__orch__commit`,
  `mcp__orch__request_pr` (the core applies its own rules to them).
- **Command line**: `--settings`, `--mcp-config`, `[--strict-mcp-config]`, `[--setting-sources]`,
  `[--model]`, `[--effort]`, `--permission-mode` (always), `--append-system-prompt` (ORCH preamble, then
  the role prompt), `-n <id>`, then `--session-id <uuid>` | `--resume <id>` | `--resume <id> --fork-session`.
  Never `--bare`, `--safe-mode` or `bypassPermissions` (not representable in `PermissionMode`).

## Environment

Nothing is inherited. `HOME USER LOGNAME SHELL LANG TERM=xterm-256color COLORTERM PATH TMPDIR`,
`ORCH_AGENT_ID ORCH_SOCK ORCH_AGENT_DIR ORCH_HOOK_DEADLINE_MS [ORCH_USER_STATUSLINE]`,
`DISABLE_AUTOUPDATER=1`, `GIT_CONFIG_GLOBAL=<agent dir>/gitconfig` (the user's name/email, empty
credential helper), `GIT_TERMINAL_PROMPT=0`, and with a receiver (the bundled one, or `--otel URL`)
`CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_*` (http/json, no compression) and
`OTEL_RESOURCE_ATTRIBUTES=orch.agent_id=<id>`.
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
- PermissionRequest: see "Inbox and permissions" below.
- The usage collector sends the Governor only **new responses** (readings or cost changed), with
  `ts` = receive time and the cost delta from `cost.total_cost_usd`. The first weekly reading
  re-aligns the plan week on `seven_day.resets_at` (`new_week`).
- Shadow mode (FR-812) is the default: decisions are logged with the proposed verdict.

## Inbox and permissions (FR-705, FR-707–709)

- **PermissionRequest**: the store's rules decide first (`deny` > `ask` > `allow`, scopes project /
  role / agent, syntax `Tool`, `Tool(exact)`, `Tool(prefix:*)`, `Tool(dir/**)`, `mcp__x__*`). With no
  deciding rule (or `ask`), an Inbox item `permission` (priority 30, payload: tool, input,
  `always_pattern`, project, role) is created and the hook is **held** (a `gate` hold row with the
  item id) for `permission_hold` (default 300 s, always 1 s inside orch-hook's deadline). Approve →
  `allow`; Reject → `deny` "the user denied this in the Inbox"; Reply → `deny` with the user's words;
  Always allow → `allow` + a scoped rule (default project) created from `always_pattern`. Esc in the
  pane (EOF on the hook connection) closes the item as `interrupted`; no answer in time closes it as
  `expired` and returns **no decision**, so the CLI's own dialog appears in the pane.
- `decide(item, InboxDecision, scope, message)` is logged in the audit log before it acts. Alerts
  are dismissed; a Reply to a non-permission item becomes a turn for the agent.
- **PR gate**: `request_pr` (orch-mcp) creates a `gate` item (one pending per agent). Approve →
  `git push --set-upstream origin <branch>` and `gh pr create` with the user's credentials (a
  thread), audited, then the agent gets "ORCH: … opened <url>". Reject → the agent is told.
- Every rule decision, user decision, rule change, commit and PR is in the audit log (FR-709).

## Commits for agents (FR-702 gap, R10)

Sandboxed git can't write `<repo>/.git/worktrees/<name>/` under 2.1.302, so agents commit through
`mcp__orch__commit(message, paths?)`: orch-mcp → bus → `Runtime::on_mcp` → rules (`deny` refuses,
`ask` holds for an Inbox approval, otherwise it runs) → `orch_git::commit::commit_in_worktree` in
that agent's worktree with its generated gitconfig, hooks off. The result (sha, branch, files) goes
back to the model as text; the audit log records actor (`agent:<id>`, `rule:<n>` or `user`),
decision and files. `Runtime::commit_for(agent, req, "user")` is the same path for the UI.

## Live cap and idle suspend (FR-203, FR-204)

`housekeeping()` (every 5 s): reads macOS memory pressure (`kern.memorystatus_vm_pressure_level`,
behind `MemoryProbe`); the live cap is `base_cap` when normal, one less on warn (≥ 1), 1 when
critical, and recovers when the pressure ends. Idle agents (no queued turn, no background task, not
attached) are suspended after `idle_suspend` (default 10 min), and while more agents are live than
the cap, the least recently active idle ones are suspended too. **Working agents are never
stopped.** A suspended agent with queued work resumes only when a live slot is free (`cap_wait`
event once). `note_user_activity(id)` resets the idle clock (pane focus).

## OTLP receiver (FR-801)

`start_otlp(port)` binds 127.0.0.1 (ephemeral by default) and points agents spawned afterwards at
it. `claude_code.api_request` log events become `otel` usage samples (tokens in/out/cache, cost,
model) attributed by `orch.agent_id`; metrics and traces are accepted and dropped. Protobuf and
compressed bodies get 415.

## Persistence and restart (NFR-2)

- The runtime's rows (agents, worktrees, turns, holds, Inbox) live in `state/orch.db`, written per
  change; a `state/state.json` from the JSON stand-in is imported once (`state.json.imported`).
- Events go to the `events` table (30-day retention) instead of `events.jsonl`.
- The Governor at start: the active plan if its week is still running (else the default week), the
  stored policy (live cap from the config), and this week's statusline samples **replayed with the
  cost class and allocation they had** (`Governor::replay`), so W, the governed share and the
  calibration survive a restart. Each tick saves the last 50 actions and the queue; a forecast
  snapshot every 5 min, the calibration hourly; retention purge daily.
- Transcripts of known agents and registered projects are indexed for search (FTS5) at start,
  then after each turn end (at most once a minute).

## Reconciler (J10 step 2)

`reconcile(&ReconcileEnv)` is a dry run with `apply: false`. With `apply: true`, in order:
1. replay each agent's `spool.jsonl` through the state machine, then rename it `*.replayed.<ts>`;
2. worktrees: finish `creating` rows (add + lock + row); a row whose folder exists but whose links
   are broken gets a **targeted `git worktree repair` + re-lock**; a project whose checkout is gone
   raises one "relocate it" alert; report `missing` (Inbox) and `unknown`;
3. merge/rebase/cherry-pick/revert in progress → **flag** (error + Inbox alert);
4. remove a stale `index.lock` when no git process has its cwd in the worktree (and after killing one);
5. kill leftovers by pgid, argv (agent dir) and cwd;
6. transcript by session id: busy states, unfinished tool uses or running background tasks →
   **resume** (a queued `ORCH: the orchestrator restarted…` turn, durable across another crash);
   else **suspend**; turns whose marker is in the transcript are `delivered`, others go back to the
   queue (and through the Governor again);
7. held gates → pending Inbox item (a permission hold closes its own item as `interrupted`: the
   agent was denied by orch-hook); held governor holds → `interrupted` (the resume continues);
   queued turns waiting in the old core's Governor queue are asked again.

An agent whose worktree is missing is flagged (error + Inbox), never resumed blindly.
`Runtime::relocate_project(id, new_path)` handles a moved repo: rows and agents are pointed at the
new place, the known worktrees are repaired (targeted) and re-locked.

## Readings of the spec (simplest reading taken)

- **Persistence**: SQLite through orch-store, with the runtime's rows mirrored in memory (written
  through per change). Other writers change those rows through the runtime only.
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

## More readings (P1 core services)

- **Commit default**: `mcp__orch__commit` is allowed in the generated settings and runs unless a
  rule says `deny`/`ask` (committing in its own worktree is the agent's normal work, FR-701).
- **Hooks off for agent commits**: a pre-commit hook would run agent-editable code outside the
  sandbox (SPEC FR-702, ADR-009 decision 5).
- **Permission hold** default 300 s: long enough to answer from the Inbox, then the pane's own
  dialog takes over (FR-705 leaves the length open).
- **Live cap**: user-started or user-resumed agents may exceed the cap (the user's action wins);
  automatic resumes wait for a slot, and pressure only ever suspends idle agents.
- **Idle**: "attached" (a terminal attached through `attach`) protects an agent; the UI marks focus
  with `note_user_activity`.

## Known gaps

- **Commits from a sandboxed worktree fail** (worked around: the core commits, see above) (measured 2026-10-21, Claude Code 2.1.302): sandboxed
  Bash cannot create `<repo>/.git/worktrees/<name>/index.lock`, whatever `allowWrite` says (plain
  or `//` spelling, the whole `.git` or its subfolders), with relative or absolute worktree links,
  with no read-deny on the checkout, and with `--add-dir`. The docs say linked worktrees get this
  grant automatically. The same runs show the CLI also blocks reads outside the working directory
  by default now (`<repo>/README.md`, `<repo>/.git/worktrees/`), which P0 did not see on the same
  version: probably a server-side switch. `allowRead` on the git dir restores reads. Until this is
  resolved, agents can edit but not commit; the core (or a gated MCP tool) has to commit for them.
- No Stop "continue" with the next task (P2 team runtime); gates other than permissions and the PR
  gate (spec gate, budget asks) are P2.
- The Week planner UI (FR-802) is not here: `set_plan` takes a confirmed `WeeklyPlan`.
- Calibration carries over a restart only through this week's replayed samples (a restart in a new
  week starts from the prior).

## Tests

- Unit tests per module, including fixture worktrees/transcripts/spools for the reconciler and the
  P0 recovery hook samples replayed through the state machine.
- `tests/e2e_fake_claude.rs`: real bus server, PTYs and helper binaries, with `tests/fake_claude.sh`
  in place of `claude` (no quota): start, delivery (marker, `/` escape), usage → Governor, a commit
  through the real orch-mcp, suspend, resume (`--resume`), kill; memory pressure (cap 1, LRU idle
  agent suspended, queued work waits for a slot, resumes when pressure ends); idle auto-suspend and
  resume on new work; orch-hook with the core down.
- Unit tests for the Inbox (rules, approve, reply, always-allow, Esc, expiry), orch-mcp tools (commit
  with deny/ask rules, PR gate), restart (Governor rebuilt from stored samples), OTLP, and the
  reconciler (repair, missing project, stale queue entries, permission items).
- `orchd/tests/crash_restart.rs`: `kill -9` of `orchd serve` mid-turn, restart, resume by id, no
  double delivery.
- `tests/live_claude.rs` (opt-in, uses quota):
  `ORCH_LIVE_CLAUDE=1 cargo test -p agent-runtime --test live_claude -- --ignored --nocapture`.
  It stops after the first response if the weekly window is above 90 %. See the file header for
  `ORCH_LIVE_REPO` and `ORCH_LIVE_ANSWER_TRUST`.

## Rust API for the UI (stable for P1)

The Tauri core embeds `Runtime` directly (no socket needed for its own calls; the bus stays for
helpers and `orchd`).

```rust,ignore
let mut cfg = RuntimeConfig::new(AppPaths::new(AppPaths::default_root()));
let rt = Runtime::new(cfg)?;                 // opens state/orch.db, restores the Governor
rt.start_otlp(0)?;                           // local receiver for agents spawned from now on
rt.recover(true)?;                           // J10 reconciler, then resumes
std::thread::spawn({ let rt = rt.clone(); move || server::serve(rt, bus::bind_listener(&sock)?, stop) });
```

| Area | Calls |
|---|---|
| Agents | `start(&AgentSpec)`, `resume`, `suspend`, `kill`, `interrupt`, `deliver`, `attach`/`read_output`/`resize`, `note_user_activity`, `state_json`, `live_ids`, `live_cap` |
| Governor | `usage_json`, `governor_json`, `pause_all`, `resume_all`, `set_plan`, `tick` (the server loop calls it) |
| Inbox | `inbox()`, `decide(id, InboxDecision, scope, message)`, `add_rule`, `revoke_rule` (audited); `rt.db.rules()` to list |
| Commits | `commit_for(agent, &CommitRequest, "user")` |
| Projects | `add_project`, `clone_project`, `projects_overview`, `project_detail(id, with_prs)`, `fetch_project`, `diff`, `remove_worktree`, `removal_plan`, `remove_project`, `relocate_project` |
| Data | `rt.db` (`orch_store::Store`): `events`, `audit_log`, `search_transcripts`, `usage_since`, `forecasts_since`, `actions`, `terminals`… (read freely; change runtime-owned tables through `rt`) |

Every control request on the bus (`orch_protocol::CtlRequest`) maps to one of these calls
(`server::ctl`), so the UI can also drive a separate core process.
