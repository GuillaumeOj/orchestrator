# P0 spike: `claude` CLI control and signal channels (SPEC §11 items 2–4)

- Date: 2026-10-17
- Claude Code: **2.1.296 at start; it auto-updated itself to 2.1.302 during the first run**. Every later run used 2.1.302 with `DISABLE_AUTOUPDATER=1`.
- Machine: macOS 27.0 (Darwin 27.0.0), arm64, 8 GB RAM. The machine was **under memory pressure**: about 6 GB of swap in use and 9 other `claude` processes from the desktop app.
- Plan: Claude Max, logged in through the keychain. `ANTHROPIC_API_KEY` was never set. All runs used `--model haiku` to save quota. The weekly window was already at 83 %.
- Harness: [spikes/claude-cli/](../../spikes/claude-cli/README.md). Each agent was a real interactive `claude` in a PTY (Python `pty`), spawned with **`env -i`** (only HOME, USER, PATH, TERM, LANG, SHELL, plus spike variables), as the app would do it. Hooks, statusline and OTEL went to logging scripts. Scrubbed sample payloads are in `spikes/claude-cli/samples/`.
- About 18 short sessions in total. Each cost about $0.01–0.02 (API-equivalent, from the statusline).

## Summary

| # | Item | Result | One-line finding |
|---|---|---|---|
| 1 | CLI flags | **PASS** | All 7 flags exist. `--permission-mode` has no `default` value any more (it is `manual`). The CLI auto-updates itself mid-session unless disabled. |
| 2 | Hooks from a per-agent `--settings` file | **PASS, with 2 caveats** | All events fire with rich payloads. (a) **A hook that exceeds its `timeout` fails open: the tool runs.** (b) Without a system-prompt note, the model treats `additionalContext` as untrusted and ignores it. |
| 3 | Statusline | **PASS** | `rate_limits.five_hour/seven_day.{used_percentage,resets_at}`, `context_window`, `cost` and `prompt_cache` are all there. `rate_limits` is missing until the first API response. `statusLine.refreshInterval` keeps it refreshing while idle. |
| 4 | OpenTelemetry | **PASS** | OTLP http/json to a local receiver works: 6 metrics and 11 event types arrived. `OTEL_RESOURCE_ATTRIBUTES` tags the agent id. No rate-limit data in OTEL. |
| 5 | Suspend/resume | **PASS** | SIGKILL while idle and while mid-tool, then `--resume <id>`: same session id and full context (`SessionStart.source = "resume"`), with 99 % prompt-cache hit inside the 1 h TTL. Background tool shells **survive** the kill. |
| 6 | PTY input while idle | **PASS, with caveats** | 37 of 38 messages delivered; the one miss started with `/`, which the CLI runs as a command. Other caveats: tabs become spaces, and **bracketed pastes of about 1000 chars or more are wrapped in `<pasted_content>`**, which the model treats as untrusted. |
| 7 | Memory | **MEASURED** | phys_footprint: **165–210 MB idle, 230–255 MB working**. RSS is misleading under pressure (66–315 MB for the same process). |
| 8 | Sandbox, no credentials | **FAIL as ADR-009 is written, PASS once hardened** | Bash writes and unlisted network were blocked. But with the env scrubbed, **the agent could still log in to GitHub** (SSH key on disk, `gh` token). Reading other worktrees was allowed. Single-slash absolute paths in permission rules **silently don't match**. All of these were fixed by `sandbox.filesystem.denyRead` plus `//` paths. |

## 1. CLI flags (2.1.302 `claude --help`)

| Flag | Exists | Syntax / notes |
|---|---|---|
| `--settings` | yes | `--settings <file-or-json>`. Adds a settings layer that loads alongside user/project/local. Its hooks and statusLine apply even with `--setting-sources project,local`. Its `statusLine` **replaces** the user's (verified). |
| `--mcp-config` | yes | `--mcp-config <configs...>` (files or JSON strings). The stdio server inherited the agent env (`ORCH_AGENT_ID` seen by the server). `--strict-mcp-config` ignores all other MCP sources. |
| `--model` | yes | Alias (`haiku`, `sonnet`, `opus`, `fable`) or full name. |
| `--permission-mode` | yes | Choices: `acceptEdits, auto, bypassPermissions, manual, dontAsk, plan`. There is **no `default`**; `manual` is reported as `"permission_mode": "default"` in hook payloads. If the flag is omitted, the **user's `defaultMode` applies** (here `auto`, so the server-side auto-mode classifier denied a Write). |
| `--append-system-prompt` | yes | Works (the agent reported its injected codename). `--system-prompt-snapshot on` (the default) records the prompt on the first request and reuses it on resume. |
| `--resume` | yes | `-r, --resume [id]`. `--fork-session` makes a new id instead. |
| `--session-id` | yes | `--session-id <uuid>` (must be a UUID). The id shows up in hooks, statusline, OTEL `session.id` and the transcript filename. |

Other flags relevant to the app: `--setting-sources`, `--strict-mcp-config`, `--tools`, `--allowed-tools`, `--disallowed-tools`, `--add-dir`, `--effort`, `-n/--name`, `--agents`, `--plugin-dir`, `--fallback-model`. **Never** use `--bare` or `--safe-mode`: both turn off hooks, including the app's own (verified for `--safe-mode`; `--bare` also skips OAuth).

Environment findings:
- **Auto-update**: the CLI replaced `~/.local/bin/claude` (2.1.296 → 2.1.302) while run 1 was going, and showed "Update installed · Restart to update". Agents need `DISABLE_AUTOUPDATER=1`, and the app must check the version range at spawn (NFR-7).
- **Workspace trust dialog**: the first launch in an untrusted folder blocks on "Quick safety check … ❯ No, exit / Yes, I trust this folder", **before any hook fires**. The default choice is *No, exit*. Answering it writes trust to `~/.claude.json`. A **subfolder** of a trusted folder did not prompt (tested with a plain subfolder, not a separate git worktree).
- **No per-agent config dir**: `CLAUDE_CONFIG_DIR=<dir>` gives "Not logged in". So all agents share `~/.claude`, whose transcripts, `~/.claude.json` and history the CLI writes itself. The app must only *read* these.
- **User customizations load into every agent**: plugins and their hooks (a plugin SessionEnd hook failed with `node: command not found` until nvm's node was added to PATH), skills, claude.ai connectors, user MCP servers, `permissions` and `defaultMode`. The agent's PATH must come from the user's login shell.

## 2. Hooks (per-agent settings file, not `~/.claude`)

Registered: SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Notification, Stop, SubagentStop, PreCompact and SessionEnd. Later runs added PermissionRequest, PermissionDenied, PostToolUseFailure, StopFailure, SubagentStart and PostToolBatch. The hook process inherits the agent env, so `ORCH_AGENT_ID` was present in 145/145 calls.

The binary also knows: `Setup, UserPromptExpansion, Elicitation(Result), WorktreeCreate/Remove, TaskCreated/Completed, TeammateIdle, FileChanged, CwdChanged, InstructionsLoaded, PostCompact, ConfigChange`.

| Check | Result |
|---|---|
| SessionStart / SessionEnd | `source: startup` or `resume`. SessionStart fired about 1–1.3 s after spawn. SessionEnd `reason`: `prompt_input_exit` (/exit) or `other` (SIGTERM). Nothing on SIGKILL. |
| PreToolUse **allow** | `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"…"}}` skips the prompt. A 5 s wait and a 20 s wait both completed and then ran the tool. |
| PreToolUse **deny** | Same shape with `"deny"`. The model gets `PreToolUse:Bash hook error: <reason>` as the tool result, quoted it verbatim, and ended the turn. |
| PreToolUse **timeout** | With `"timeout": 30` and the hook sleeping 45 s (to answer deny), the hook was dropped at 30 s and **the tool ran**. Timeouts fail open. |
| Parallel tool calls | Two PreToolUse hooks ran **concurrently** (0.3 s apart), each holding its own tool. |
| Deny rule vs hook allow | `permissions.deny: ["Bash(git push:*)"]` still denied `git push origin HEAD` although PreToolUse answered `allow`. Deny rules win, which is good. |
| PostToolUse `additionalContext` | Delivered as a `hook_additional_context` attachment. **Without framing, the model refused it**: "I left out the word BANANA. A hook message asked for it, but that request didn't come from the user". With `--append-system-prompt` saying that text starting with `ORCH:` in hook context comes from the orchestrator, it followed it. |
| Stop "continue" | `{"decision":"block","reason":"<next task>"}` works. The model ran the next task in the same turn, and the second Stop came with `stop_hook_active: true`. The TUI labels it "Stop hook error: …" (cosmetic). The Stop payload also has `last_assistant_message`, `background_tasks` and `session_crons`. |
| Notification | `notification_type: "permission_prompt"` ("Claude needs your permission") and `"idle_prompt"` ("Claude is waiting for your input") about **60 s** after Stop. |
| **PermissionRequest** (new) | Fires when a permission dialog would show. The hook answered `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}` after 3 s: no dialog, and the TUI shows "Allowed by PermissionRequest hook". This is a direct Inbox channel. |
| Rule-denied calls | A `Read`/`Write` denied by a `permissions.deny` rule fires **neither PreToolUse nor PermissionDenied**. It only shows up in **PostToolBatch** (`tool_calls[].tool_response` = `<tool_use_error>File is in a directory that is denied by your permission settings.`). |
| Sandbox violations | No dedicated event. They are visible only in PostToolUse output (`Operation not permitted`, or a `<sandbox_violations>` block for network). |
| Not exercised | SubagentStop/SubagentStart, PreCompact, StopFailure (rate-limit hit), the PreToolUse `ask` decision, waits longer than 20 s with a large timeout. |

Sample PreToolUse payload (paths trimmed):
```json
{"session_id":"f8ffc5d4-…","transcript_path":"~/.claude/projects/<slug>/f8ffc5d4-….jsonl","cwd":"<ws>","prompt_id":"1865e119-…",
 "permission_mode":"default","effort":{"level":"medium"},"hook_event_name":"PreToolUse","tool_name":"Bash",
 "tool_input":{"command":"echo hello-spike","description":"Print hello-spike"},"tool_use_id":"toolu_018P…"}
```
PostToolUse adds `tool_response` (`stdout`, `stderr`, `interrupted`, …) and `duration_ms`. All event samples are in `samples/hook-payloads.json`.

## 3. Statusline

- The command gets the JSON on stdin and its stdout is shown. The per-agent `statusLine` replaced the user's own (`~/.claude/statusline-command.sh`), so `orch-status` must chain to it.
- **The first call (right after start) has no `rate_limits`** and has `cost.total_cost_usd: 0`. From the **first API response on**, every call has:
```json
"rate_limits":{"five_hour":{"used_percentage":7,"resets_at":1792270200},"seven_day":{"used_percentage":83,"resets_at":1792378800}},
"context_window":{"total_input_tokens":65798,"total_output_tokens":443,"context_window_size":1000000,
  "current_usage":{"input_tokens":2,"output_tokens":443,"cache_creation_input_tokens":343,"cache_read_input_tokens":65453},
  "used_percentage":7,"remaining_percentage":93},
"cost":{"total_cost_usd":0.0137,"total_duration_ms":44450,"total_api_duration_ms":21624,"total_lines_added":0,"total_lines_removed":0},
"prompt_cache":{"warm":true,"ttl":"1h","expires_at":1792267440,"hit_ratio":0.92,"recache_tokens_if_cold":65798,…}
```
  - `used_percentage` is an integer.
  - `resets_at` is in epoch seconds.
  - There is no separate per-model weekly field in this payload.
  - Other fields: `session_id`, `session_name`, `model`, `effort`, `version`, `workspace`, `exceeds_200k_tokens`, `fast_mode`, `thinking`.
- **Update cadence**: by default it runs on activity only (12 calls in a 45 s busy period, **none in the following 70 s idle**). With **`"statusLine": {…, "refreshInterval": 5}`** it re-ran every 5.0 s while idle. Whether idle refreshes return *newer* `rate_limits` was not proven: no other usage happened during the test.
- `prompt_cache` (`ttl`, `expires_at`, `recache_tokens_if_cold`) is a ready-made input for the resume-cost model (R6).

## 4. OpenTelemetry

Settings: `CLAUDE_CODE_ENABLE_TELEMETRY=1 OTEL_METRICS_EXPORTER=otlp OTEL_LOGS_EXPORTER=otlp OTEL_EXPORTER_OTLP_PROTOCOL=http/json OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318 OTEL_METRIC_EXPORT_INTERVAL=5000 OTEL_LOGS_EXPORT_INTERVAL=2000`. Data arrived on `/v1/metrics` and `/v1/logs`.

- **Metrics**: `claude_code.session.count`, `active_time.total`, `cost.usage`, `token.usage` (attributes `model`, `type`, `query_source`, `effort`), `code_edit_tool.decision`, `lines_of_code.count`.
- **Log events**: `api_request` (`model`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_creation_tokens`, `cost_usd`, `duration_ms`, `ttft_ms`, `query_source`), `tool_result`, `tool_decision`, `user_prompt` (the prompt text is not included by default), `assistant_response`, `hook_registered`/`hook_execution_start|complete`, `mcp_server_connection`, `plugin_loaded`, `managed_settings_resolved`.
- Every record carries `session.id`. `OTEL_RESOURCE_ATTRIBUTES=orch.agent_id=…` showed up as a resource attribute. Records also carry `user.email`, account and organization ids, so the receiver must stay local-only.
- No `rate_limits` in OTEL: the statusline remains the only source of usage %.

## 5. Suspend / resume

| Case | Result |
|---|---|
| SIGKILL while idle, then `claude --resume <id>` | Same `session_id`; `SessionStart.source = "resume"`. Asked "what did you run earlier and what's your codename", it answered correctly. The first request after resume had a 0.99 cache hit ratio (within the 1 h TTL). |
| SIGKILL **mid-tool** (`sleep 25 && echo slept > slept.txt`), then resume | It resumed with the note "Background shell command didn't finish before the previous session ended". The agent checked `slept.txt`, saw step 1 had finished, ran step 2 only, and said ALL DONE. |
| Orphans | The backgrounded shell **kept running after SIGKILL** of `claude` and wrote `slept.txt` about 20 s later. Kill must target the process group or session and its descendants. |
| PTY input while busy | Text pasted during a turn was **queued by the CLI** (`queue-operation enqueue/dequeue` in the transcript) and delivered as the next turn. |

## 6. PTY input (bracketed paste + Enter)

- **Delivery**: 38 messages were sent across all runs (37 bracketed pastes, 1 typed) and 37 produced a `UserPromptSubmit`. The one miss is the leading-`/` case below. Paste-to-`Stop` time for a trivial reply was about 1.5–2 s. Multi-line text, unicode, emoji, quotes, `$HOME` and backticks all arrived byte-exact.
- **Leading `/`** is run as a slash command or skill ("Unknown command" / "Args from unknown skill"). There is **no UserPromptSubmit and no Stop**, so the app would wait forever. A leading space avoids it; ` !…` and ` #…` with a leading space were also safe. Without the space, `!` would run a shell command directly.
- **Tabs** became 4 spaces.
- **Bracketed paste of ≥ ~1000 chars** (487 chars: verbatim; 988 chars: wrapped) reaches the model as `<pasted_content id="…">…</pasted_content>`, which the system prompt says to treat as untrusted data. **Typing the same 2848 chars without bracketed paste** arrived verbatim. Typed text cannot contain newlines, though: Enter submits.
- **Startup**: the TUI dropped keys sent about 0.8 s after the trust dialog first rendered. Waiting 1.5 s fixed it. Gate delivery on SessionStart plus a short delay.

## 7. Memory (phys_footprint from `footprint -p`, main `claude` process)

| State | Config | Footprint |
|---|---|---|
| Idle, fresh, 10–20 s after start | default (user plugins, connectors, user MCP) | 194–208 MB |
| Idle, fresh | `--strict-mcp-config` | 174–176 MB |
| Idle, fresh | `--strict-mcp-config --setting-sources project,local` | 168–174 MB |
| Idle, fresh | `--safe-mode` (floor; unusable, hooks off) | 163–164 MB |
| Working (mid-tool) | default | 230 MB |
| Idle after resume and one turn | default | 242–254 MB |
| Long-lived desktop-app `claude` sessions (for comparison) | — | 180–231 MB |

RSS for the same process ranged from 66 MB to 315 MB depending on compression and swap, so RSS should not be used as the budget metric. Child processes such as MCP servers and hook scripts are short-lived or small (python stub: about 10–17 MB). Planning figure: **about 250 MB per live agent**, so 3 agents ≈ 0.75 GB.

## 8. Sandbox and credentials

Fixture: a git repo as the agent's cwd, an `outside/` folder, and a fake "other worktree" holding a secret. The remotes point to a **nonexistent** GitHub repo, so nothing was ever pushed. A `probe.sh` run through the Bash tool tests each vector.

| Vector | No sandbox (env scrubbed) | ADR-009 as written¹ | Hardened² |
|---|---|---|---|
| Bash write inside cwd | ok | ok | ok |
| Bash write to `outside/`, `$HOME`, `/tmp` | **written** | blocked ("Operation not permitted") | blocked |
| Bash read of another worktree | **read** | **read** | blocked |
| Bash read of `~/.ssh/id_ed25519`, `~/.config/gh/hosts.yml` | **readable** | **readable** | blocked |
| `curl https://example.com` (unlisted) | 200 | blocked (proxy 403 + `<sandbox_violations>`) | blocked |
| `curl https://registry.npmjs.org/` (listed) | 200 | 200 | 200 |
| `ssh -T git@github.com` | **"Hi GuillaumeOj! You've successfully authenticated"** | blocked (no DNS for non-HTTP) | blocked |
| `gh api user` | **"GuillaumeOj"** (token from keyring) | failed (proxy connect refused on `::1`) | failed (config unreadable) |
| `git push` (ssh, inside a script) | reached GitHub: "Repository not found" | failed | failed |
| `git push` (https, `GIT_TERMINAL_PROMPT=0`) | auth attempted, then failed | failed: no username | failed |
| Top-level `git push origin HEAD` (Bash tool) | — | denied by `Bash(git push:*)` | denied |
| `Read` tool on the other worktree | — | **READ-OK** (rule written `Read(/Users/…)`) | denied with `Read(//Users/…/**)` |
| `Write` tool to `outside/` | — | denied, but by the user's auto-mode classifier rather than the rule (same `/` bug) | denied with `Edit(//Users/…/**)` |

¹ `sandbox.enabled`, `failIfUnavailable`, `autoAllowBashIfSandboxed`, `allowUnsandboxedCommands: false`, `network.allowedDomains: [registry.npmjs.org, github.com, api.github.com]`, plus permission deny rules with single-slash absolute paths.
² Adds `sandbox.filesystem.denyRead: ["~/.ssh", "~/.config/gh", "<other worktree>"]` and deny rules with `//` absolute paths. Adding `~/.gitconfig` to `denyRead` **broke git entirely** ("unable to access '~/.gitconfig'"), so don't do that: point `GIT_CONFIG_GLOBAL` at a generated file instead.

Takeaways:
- **Removing credentials from the env is not enough.** On this machine the SSH key on disk and the `gh` keyring token both reach GitHub.
- The sandbox's **read** side is open by default.
- The `Read` and `Write` tools are **not** covered by the sandbox, only by permission rules. Absolute paths in those rules need `//`; a single `/` is relative to the settings file and silently matches nothing.
- A `Bash(git push:*)` rule only catches a *top-level* `git push`. The same push inside a script is stopped only by the credential and network layers.

## Proposed SPEC / ADR corrections (not applied)

1. **SPEC §6.3 (flags)**:
   - Confirm the seven flags.
   - Change the permission mode list to `acceptEdits | auto | bypassPermissions | manual | dontAsk | plan`, and always pass it explicitly. Otherwise the user's `defaultMode` (e.g. `auto` with its server-side classifier) applies to agents.
   - Add `--strict-mcp-config`, `--setting-sources`, `-n/--name` and `--effort` to the generated command line.
   - Forbid `--bare` and `--safe-mode`, because both disable hooks.
2. **SPEC §6.3 / ADR-002 (environment)**:
   - Add `DISABLE_AUTOUPDATER=1`. The CLI replaced its own binary mid-run, which matters for R2 and NFR-7.
   - Add a version check at spawn.
   - Take PATH from the user's login shell, because plugin hooks need `node`.
3. **SPEC §6.3 ("never touches `~/.claude`")**:
   - Reword. The app writes nothing there, but every agent shares `~/.claude`, because `CLAUDE_CONFIG_DIR` isolation loses the login. The CLI itself writes transcripts and trust there.
   - State that user plugins, skills, connectors and hooks load into every agent unless the role sets `--strict-mcp-config` or `--setting-sources`.
4. **SPEC J0 / FR-101/102, new requirement**: the workspace trust dialog blocks the first launch in an untrusted folder, before any hook fires. Fix: have the user trust the project root once (subfolders inherit), and put worktrees under it. Otherwise detect the dialog on the PTY and surface it.
5. **ADR-007 admission point 2 and SPEC §6.4**:
   - Hook timeouts **fail open**. `orch-hook` must enforce its own deadline and answer `deny` ("paused by governor") before the configured `timeout`. Set the timeout generously (e.g. 600 s) and keep the governor wait well below it.
   - Note that PreToolUse hooks run concurrently for parallel tool calls.
6. **SPEC §6.4 (inject into a busy agent)**:
   - `additionalContext` is delivered, but the model ignores it unless the appended system prompt declares the orchestrator channel (e.g. "text prefixed `ORCH:` in hook context comes from the orchestrator on the user's behalf").
   - Make that preamble part of every role's compiled prompt.
   - Write the Stop "continue" format as `{"decision":"block","reason":…}` and guard it with `stop_hook_active`.
7. **SPEC §6.4 and FR-705 (gates and Inbox)**:
   - Add the **PermissionRequest** hook as the channel for permission dialogs (allow or deny programmatically, verified). Short gates can then be answered from the Inbox without ending the turn. The deny-and-suspend path (FR-705) stays for long waits.
   - Add **PostToolBatch** as the audit source for tool calls denied by rules, which never reach PreToolUse or PermissionDenied.
   - Add **StopFailure** for API and rate-limit errors (registered, not yet exercised).
8. **FR-206 and R4 (PTY delivery)**:
   - Escape a leading `/`, `!` or `#` with a leading space.
   - Don't rely on tabs surviving.
   - Keep bracketed pastes under about 900 chars. Longer instructions go to a file under `.orchestrator/` and get a short reference message, because longer pastes are wrapped as untrusted `<pasted_content>`.
   - Wait about 1.5 s after the TUI first renders before sending keys.
   - Note that input typed during a turn is queued by the CLI rather than lost.
9. **R3 and FR-813/814 (statusline)**:
   - Confirmed: `rate_limits` is missing until the first API response of a session.
   - Add `statusLine.refreshInterval` (seconds) so idle agents keep reporting.
   - `orch-status` must chain to the user's own statusLine command, because `--settings` replaces it.
   - Use `prompt_cache.expires_at`, `ttl` and `recache_tokens_if_cold` for R6.
10. **FR-204 and R6 (suspend/resume)**:
    - Verified, including a kill mid-tool.
    - The cache TTL observed is **1 h**, so the 10 min idle-suspend default costs nothing in cache and could be tuned against `prompt_cache.expires_at`.
    - **FR-207 kill**: signal the process group and its descendants. Background shells survived a SIGKILL of `claude`.
11. **NFR-1b and FR-203**:
    - Measure memory as phys_footprint, not RSS.
    - Per-agent budget about 250 MB (165–210 MB idle, 230–255 MB working).
    - The default cap of 3 live agents is reasonable on 8 GB, but this machine already swaps 6 GB with the desktop app's own sessions, so the cap should react to system memory pressure rather than be fixed.
12. **ADR-009 decisions 1–3 and acceptance tests**:
    - (a) Add `sandbox.filesystem.denyRead` for credential stores (`~/.ssh`, `~/.config/gh`, `~/.netrc`, `~/.git-credentials`, `~/.aws`, …) and for the main checkout and sibling worktrees. Without it, "no credentials" and "read another worktree" both fail.
    - (b) Don't deny `~/.gitconfig`. Set `GIT_CONFIG_GLOBAL` to a generated file with no credential helper, plus `GIT_TERMINAL_PROMPT=0`.
    - (c) Write permission-rule absolute paths as `//abs/path`.
    - (d) Say that the `Read`/`Edit`/`Write` tools are guarded only by permission rules, not the OS sandbox.
    - (e) Say that a `Bash(git push:*)` rule is defense in depth only: scripts bypass it.
    - (f) Note that deny rules beat a hook's `allow`.
    - (g) Note that the sandbox blocks `/tmp`; temp writes go to the CLI's own scratch dir.
    - (h) Untested: keychain access from inside the sandbox (`gh` failed earlier for proxy reasons), Docker and browsers, and whether the model attempts `dangerouslyDisableSandbox` with `allowUnsandboxedCommands: false`.
13. **SPEC §6.4 table (OTEL row)**:
    - List the actual metrics and events.
    - Tag each agent with `OTEL_RESOURCE_ATTRIBUTES=orch.agent_id=…`.
    - Note that records include the user's email and account ids, so keep the receiver local.

## Follow-ups not covered here

- A hook held for minutes under a large `timeout`, while the user types in the same pane.
- SubagentStop and PreCompact payloads.
- StopFailure on a real rate-limit hit.
- Several agents running at once: memory and statusline consistency.
- The trust dialog inside an actual `git worktree` directory under a trusted root.
