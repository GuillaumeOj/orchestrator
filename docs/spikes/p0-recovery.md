# P0 spike: crash/restart reconciliation and the open CLI items (SPEC §11 item 6, R10)

- Date: 2026-10-20
- Claude Code **2.1.302** (`DISABLE_AUTOUPDATER=1` on every agent), git 2.54.0 (Apple Git-157), Python 3 for the harness.
- Machine: macOS 27.0.1 (Darwin 27.0.0), arm64, 8 GB RAM, **under memory pressure for the whole run**: 27–31 % free memory, 7.0–8.0 GB of swap in use (the swap file grew from 8 to 9 GB during the run), 13–42 other `claude` processes from the desktop app.
- Plan: Claude Max through the keychain login. `ANTHROPIC_API_KEY` was never set. All agents used `--model haiku`, trivial prompts and `--strict-mcp-config`. Usage was checked on the statusline before anything else: **weekly 6 %** (the window had reset since 2026-10-17), 5h 34 % at the start and 40 % at the end (this includes the user's other sessions). About 28 agent sessions cost about **$0.10** API-equivalent in total.
- Harness: [spikes/recovery/](../../spikes/recovery/README.md). `core.py` plays the core: Unix socket (0600), SQLite (WAL), and **the agents' PTYs owned by the core process itself**, as the Tauri core would own them. Each agent is a real interactive `claude` in its own session and process group, with a from-scratch env, per-agent `--settings` (all hooks → `orch_hook.py`, `timeout: 600`; statusline → `orch_status.py`, `refreshInterval: 5`), `--session-id`, and a `git worktree lock`ed worktree in the sibling folder `demo.worktrees/solo/<id>` of a scratch repo. App-delivered turns carry a `[t:<id>]` marker so the reconciler can find them in the transcript. Scrubbed samples are in `spikes/recovery/samples/`.
- Constraints respected: nothing was written to `~/.claude` or `~/.claude.json` by the harness. Claude Code itself wrote transcripts and **5 trust entries** for the scratch paths (one per trust dialog answered through the PTY, as J0 has the user do). Every process the spike started was stopped at the end (agents, cores, the 429 mock, the hung-socket listener) and the sockets were removed.

## Summary

| # | Item | Result | One-line finding |
|---|---|---|---|
| A1a | kill -9 core, agent **idle** | **PASS** | All agents exit by themselves on the PTY hangup (SIGHUP) within ≤ 3 s. Reconciler: transcript ends on `end_turn` → **suspend**. |
| A1b | kill -9 core, agent **mid-tool** | **PASS, with a caveat** | `claude` kills its foreground shell and records `tool_result: "Exit code 137"`. Resumed, the agent saw the partial effect and re-ran the command: the orchestrator delivered nothing twice, but **a non-idempotent command leaves its partial effect twice** (`start` twice in the log). |
| A1c | kill -9 core, agent **held in PreToolUse** (gate and governor hold) | **PASS** | orch-hook saw EOF and **denied** (fail closed): the tool never ran. The gate came back as a pending Inbox item; on approval the resumed agent checked first and ran the command **exactly once**. The governor hold was requeued and also ran once. |
| A1d | kill -9 core, agent **mid-merge** | **PASS** | `MERGE_HEAD` in the worktree gitdir + 1 unmerged path → flagged, left suspended, Inbox alert. |
| A1e | Orphans | **PASS, but FR-207 is wrong** | `claude` itself never outlives the core by more than ~10 s. But **Claude's Bash tool runs every command in its own session** (`setsid`, no tty), so killing the agent's process group misses it, and a `cmd &` started by the agent **survives everything**. It was found only by its **cwd** (inside the worktree) and killed. |
| A1f | Locks | **PASS** | `git worktree lock` survives crashes and moves and keeps the worktree out of `prune`. A stale `index.lock` only appears when **git** is SIGKILLed (git cleans it up on SIGTERM/SIGHUP). It was detected (no live git in that worktree) and removed before the resume. |
| A1g | Core crash between `git worktree add` and the DB write | **PASS** | Reconciler finds the `creating` row + the git worktree and finishes creation. The lock step must be re-run (idempotent steps). |
| A2 | orch-hook with the core down | **FAIL as specified, PASS with the designed fallback** | A hook that crashes (exit 1) or **is missing** lets the tool run (fail open). `exit 2` on every event **blocks the user's own prompt**. Designed default: JSON **deny** for PreToolUse/PermissionRequest, exit 0 + spool for everything else, ≤ 1 s connect timeout. |
| A3 | Moving the repo folder | **PASS with repair** | Absolute worktree links break both ways; `git -C <repo> worktree repair <paths…>` fixes them. With **`worktree.useRelativePaths=true`** a joint move needs no repair. The trust dialog comes back once. **`--resume <id>` still finds the session** in the old project folder. |
| B4 | PreToolUse held for minutes while the user types | **PASS** | 200 s hold under a 600 s timeout: no fail-open, the TUI kept echoing input, submitted messages were **queued** and answered in the same turn after the tool ran. **Esc kills the hook and no event reports it.** |
| B5 | SubagentStart/Stop, PreCompact | **PASS, with surprises** | The Agent tool is **async by default**: Stop fires while the subagent runs, and its result **starts a new turn by itself** (`<task-notification>` UserPromptSubmit). Internal SubagentStops (empty `agent_type`, no SubagentStart) fire after Stop and during compaction. |
| B6 | StopFailure | **PASS** | Seen with `error: "model_not_found"` (invalid `--model`) and `error: "rate_limit"` (local 429 mock). On the 429 the statusline showed `five_hour.used_percentage: 100` and the TUI said **"continuing automatically at 12:32pm"**. |
| B7 | Three agents at once | **PASS** | phys_footprint 189–206 MB each (≈ 0.6 GB for 3). All three reported identical `rate_limits` at the same second. The statusline is a **per-process** value (the 429 agent read 100 % while the others read 40 %). |
| B8 | Trust dialog in a real worktree | **FAIL as J0/ADR-003 are written** | A worktree under a trusted `<repo>.worktrees/` folder **still prompts**. A worktree of a **trusted main repo does not**, wherever it lives. Trust is keyed by the main repository root. |

## A1. Crash and restart

### What happens to the agents when the core dies

The core owns the PTY masters. `kill -9` of the core closes them, the kernel sends SIGHUP to each `claude` (session leader of its PTY), and:

| Agent state at the kill | What `claude` did | Time to exit |
|---|---|---|
| idle (3 agents) | Exited; SessionEnd hook ran (`reason: "other"`) and was spooled by orch-hook. | < 3 s |
| mid-tool (`sleep 45` in the foreground) | Killed its shell, wrote `tool_result: "Exit code 137", is_error: true` to the transcript, exited. | < 3 s |
| held in PreToolUse | orch-hook got EOF on the socket and answered **deny**; the turn ended; exited. | < 3 s |
| idle with a `run_in_background` shell (`git commit` waiting on its editor) | Killed the background task (`[killed]` in its output file), exited. The staged file stayed in the worktree. | 4–10 s |
| idle after the agent ran `(sleep 300; …) &` in a Bash call | `claude` exited. **The detached `sleep 300` kept running** (ppid 1, no tty, its own process group). | — |

So with an in-process PTY owner, a core crash ends every agent: "orphaned `claude` processes" exist only for the first few seconds after a fast restart (the reconciler found one dying `claude` 1.5 s after the kill), and the real orphans are **detached children of Bash tool shells**.

Process facts that matter for FR-207 and the reconciler:
- Each Bash tool command runs as `/bin/zsh -c source ~/.claude/shell-snapshots/… && eval '<cmd>'` in **its own session and process group, with no controlling tty** (`ps` stat `SNs`, tty `??`). `killpg(claude's pgid)` does not reach it. `claude` tracks and kills these shells itself when it exits normally or on SIGHUP, but not on SIGKILL (P0 CLI spike) and never their detached `&` children.
- **Environment markers are only partly readable on macOS 27**: `KERN_PROCARGS2` returns the env of the `claude` binary (`ORCH_AGENT_ID` was readable), but **no env at all for Apple platform binaries** (`/bin/zsh`, `/bin/bash`, `sleep`). Shells can't be matched by `ORCH_AGENT_ID`.
- What did match: **process group** (the `claude` process), **argv** (the per-agent `--settings` path), and **cwd inside the agent's worktree** (`lsof -d cwd`) for the detached shell. Reconciler sample (round 3):

```json
{"id":"d2-merge","procs":[{"pid":66806,"pgid":66806,"tty":"ttys000","stat":"SNs+","how":["pgid","argv","cwd"]}],"decision":"flag"}
{"id":"g-orphan","procs":[{"pid":67817,"pgid":67815,"tty":"??","how":["cwd"]},{"pid":67818,"pgid":67815,"tty":"??","how":["cwd"]}],
 "kill_log":["killpg(67815,SIGTERM)","kill(67817,SIGTERM)","kill(67818,SIGTERM)"],"decision":"suspend"}
```

### The reconciler

On start, before opening the socket, `core.py serve --recover`:
1. **Replays the orch-hook spool** (events hooks couldn't deliver while the core was down) and applies their state transitions (Stop → idle). SessionEnd is ignored: its `reason` is `"other"` for a crash (SIGHUP) and for a suspend (SIGTERM) alike, so the core's own intent decides.
2. **Git**: `git worktree list --porcelain` against the `worktrees` table: `creating` + present → finish (and re-lock); present but unknown → flag; in DB but missing → flag (moved or removed); per worktree, markers in its gitdir (`MERGE_HEAD`, `REBASE_HEAD`, `rebase-merge/apply`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `index.lock`), `git ls-files -u` and dirty count. A stale `index.lock` (no git process with its cwd in that worktree) is removed.
3. **Processes**: by pgid, argv, env (`claude` only) and cwd; everything found is SIGTERMed by group and pid, then SIGKILLed after 5 s.
4. **Transcript** (`~/.claude/projects/*/<session_id>.jsonl`, looked up by id): last entry type and stop reason, `tool_use` without `tool_result`, and the `[t:<id>]` turn markers.
5. **Decision per agent**: merge/rebase in progress → **flag** (suspended + Inbox alert); open gate → **suspend**, gate re-created as a pending Inbox item; working, queued, unfinished tool use, or `background_tasks` still running at the last Stop → **resume** (`--resume <id>` + "ORCH: the orchestrator restarted while you were working. Check what your last step actually did before redoing anything, then finish the task."); otherwise → **suspend**.
6. **Turns**: a turn marked `delivering`/`delivered` in the DB is `delivered` if its marker is in the transcript, otherwise requeued. **No turn was delivered twice** in any round.

| Round | Agents at the kill | Reconciler decision | Outcome after restart |
|---|---|---|---|
| 1 | a-idle (idle), b-tool (mid `sleep 45`), c-gate (gate held) | suspend / resume / suspend + gate pending | b-tool: "The log shows only `start`, so the earlier run was killed… I'll run the exact command once more" → `start, start, end`. c-gate, after approval: checked `gated.txt` (absent), ran once. |
| 2 | e-bg (idle, bg shell), f-gov (governor hold, 600 s) | resume (bg task lost) / resume (hold requeued) | e-bg got Claude's own note "Background shell command didn't finish before the previous session ended", inspected, re-ran it. f-gov: "The earlier command did not run", ran once. |
| 3 (restart 0 s after the kill) | d2-merge (merge conflict, gate held on `git status`), g-orphan (detached `sleep 300`), stale `index.lock` in e-bg | kill dying `claude` + flag / kill orphan shell + suspend / remove lock, resume e-bg, f-gov | Inbox: "Merge in progress after a restart: MERGE_HEAD in worktree, 1 unmerged path(s)". e-bg's git worked again. |
| crash point | core SIGKILLs itself right after `git worktree add` | finish creation | Row `creating` → `ready`. The `git worktree lock` was missing and must be re-applied. |

## A2. orch-hook when the core is down

Same prompt each time ("run `echo probe >> probe.txt`"), agent alive, socket missing (stale socket file → `ECONNREFUSED`, missing file → `ENOENT`; both immediate) or hung.

| orch-hook behavior | Tool ran? | What else happened |
|---|---|---|
| JSON `permissionDecision: "deny"` with "ORCH: orchestrator unavailable…" (designed) | **no** | The model quoted the reason, said it would wait, ended the turn. Other events: exit 0, spooled. |
| exit 0 with no output ("open") | **yes** | — |
| crash: stderr + exit 1 | **yes** | Every hook shows "hook error: Failed with non-blocking status code" in the TUI. |
| exit 2 on every event | no | **UserPromptSubmit exit 2 blocked and erased the prompt** ("UserPromptSubmit operation blocked by hook… Original prompt: …"). SessionStart also errors. |
| **hook binary missing** (`/Applications/Orchestrator.app/…/orch-hook` absent) | **yes** | Non-blocking error "No such file or directory" on every event. |
| core hung (socket accepts, never answers), own deadline 5 s | no | Every event, including UserPromptSubmit, waited the full 5 s before falling back. |

Designed default, verified: **PreToolUse and PermissionRequest → JSON deny; every other event → exit 0 immediately and append to `<agent dir>/spool.jsonl`**; connect timeout ≤ 1 s; only PreToolUse/PermissionRequest ever wait on the core. Spool sample: `{"event":"PreToolUse","why":"core closed the connection"}`, `{"event":"SessionEnd","why":"ConnectionRefusedError"}`.

## A3. Moving the repo folder

Scratch repo with two worktrees: `m-a` (default, absolute links) and `rel` (created with `-c worktree.useRelativePaths=true`, git ≥ 2.48: `.git` file `gitdir: ../../../demo/.git/worktrees/rel`).

| Move | Absolute worktree | Relative worktree | Fix |
|---|---|---|---|
| `proj/` → `moved/` (repo and `.worktrees` together) | `fatal: not a git repository` | **still works** | `git -C <repo> worktree repair <worktree paths…>` |
| repo only (`demo` → `demo-renamed`) | broken | broken | same; without `useRelativePaths` set in the repo, repair rewrites relative links as absolute |

- Plain `git worktree repair` from the main checkout can't find moved worktrees: the reconciler must pass their new paths (from the DB, rebased on the new root).
- A **locked** worktree whose folder is gone is listed without `prunable`, so the lock protects it from `git worktree prune` during the move.
- Claude: the first `claude --resume <id>` in the moved worktree showed the **trust dialog** (the new main-repo path is untrusted). After it was answered, the session resumed with its history (`SessionStart.source: "resume"`). The transcript **stayed in the old project folder** (`~/.claude/projects/<old slug>/`) and resume found it there, so transcripts must be looked up by session id, never by a slug computed from the cwd.
- Reconciler work for a move: detect that the project path is gone, ask the user to locate it, rewrite every path in the DB, run `worktree repair` with the new paths, re-lock, then surface the trust dialog once.

## B4. Long PreToolUse hold while the user types

Hook timeout 600 s, core holding `echo hold-me >> held.txt` for 200 s then allowing.

- No fail-open: the tool ran only when the core answered (t = 201 s).
- Typing during the hold echoed immediately. Enter showed "Press up to edit queued messages" and "ctrl+x ctrl+s to send now". The transcript has `queue-operation: enqueue` for each message.
- **UserPromptSubmit fires when the message is queued**, not when it is delivered, with the **same `prompt_id`** as the running turn (t = 25 s and 53 s). It is not a new turn.
- After the tool ran, both queued messages were attached to the tool result (`queue-operation: remove`) and answered in the same turn: `DONE / ALPHA / BETA`.
- Held agent: 193 MB phys_footprint, about 1 % CPU.
- **Esc during a hold**: the TUI shows "Interrupted · What should Claude do instead?", the orch-hook process is killed within 4 s, and **no hook event fires**. The core's hold stayed "held" until it next wrote to the dead connection.

## B5. Subagents and compaction

- `Agent` tool (general-purpose subagent): PreToolUse → PostToolUse with `tool_response: {"isAsync": true, "status": "async_launched", "agentId": "…", "outputFile": "…"}` (7 ms) → SubagentStart → **Stop with `background_tasks: [{"type":"subagent","status":"running",…}]`** → SubagentStop (`last_assistant_message: "PONG"`, `agent_transcript_path`) → a **UserPromptSubmit whose prompt is a `<task-notification>`** (`<status>completed</status>`, `<result>PONG</result>`, `<usage><subagent_tokens>30725</subagent_tokens>…`) → a second Stop. The second turn was started by the CLI, not by the app.
- `SubagentStart`: `{"agent_id":"a149a5e53b20a104b","agent_type":"general-purpose","hook_event_name":"SubagentStart","prompt_id":"…"}`.
- Internal subagents fire **SubagentStop only** (no SubagentStart), with `agent_type: ""`, ~1–2 s after Stop and during compaction. Sometimes their `agent_transcript_path` doesn't exist on disk.
- `/compact` (typed): PreCompact `{"trigger":"manual","custom_instructions":null}` → SubagentStop (the summarizer, `last_assistant_message: "<summary>…"`) → **SessionStart `source: "compact"`** → PostCompact `{"trigger":"manual","compact_summary":"<summary>…"}`. 8 s in total.

## B6. StopFailure

- `--model claude-nonexistent-0`: SessionStart reports the bad model; the first prompt gives **StopFailure** `{"error":"model_not_found","last_assistant_message":"There's an issue with the selected model…"}` and no Stop. Free.
- Plan limit: the agent was started with `ANTHROPIC_BASE_URL` pointing to `mock_429.py` (local, logs only method/path/status, never headers). Two `POST /v1/messages` got a 429 with `rate_limit_error` and `anthropic-ratelimit-unified-*` headers (5h rejected, reset in 1 h). Result: **StopFailure** `{"error":"rate_limit","last_assistant_message":"You've hit your session limit · resets 12:32pm (Europe/Paris)"}`, no retries, statusline `rate_limits: {"five_hour":{"used_percentage":100,"resets_at":1792492371}}` with **no `seven_day` key**, and the TUI banner "**Usage limit reached · continuing automatically at 12:32pm · esc to cancel**". The agent was stopped before that time. A real limit hit was not exercised.

## B7. Three agents at once

| | x | y | z |
|---|---|---|---|
| phys_footprint, mid-tool | 196 MB | 202 MB | 203 MB |
| phys_footprint, idle after 1 turn | 198 MB | 206 MB | 199 MB |
| statusline `rate_limits` (11/10/10 samples, 41 s) | 5h 40 % @1792489800, 7d 6 % @1792983600 | same | same |

Round 1 gave 195 / 202 / 189 MB and identical values too (5h 34 %, 7d 6 %). Memory at the time: 29–31 % free, 7.6–7.7 GB swap used. The statusline is the last API response's headers **cached per process**: after the 429 the mocked agent read 100 % while the real agents read 38–40 %. An idle agent's `refreshInterval` re-renders its own last value, so it doesn't see other agents' usage (answers R3).

## B8. Workspace trust in a real worktree

| Trusted (answered once) | Launched in | Dialog? | Entry written to `~/.claude.json` |
|---|---|---|---|
| `t8a/proj/demo.worktrees` (a plain folder) | `t8a/proj/demo.worktrees/solo/wt-a` (git worktree) | **yes** | `…/demo.worktrees` |
| `t8b/proj/demo` (main repo) | `t8b/proj/demo.worktrees/solo/wt-b` (git worktree) | **no**, SessionStart fired directly | `…/demo` only |
| `m/proj/demo`, then the repo was moved to `m/moved/demo` | `m/moved/demo.worktrees/solo/m-a` | **yes** (once) | `…/m/moved/demo`: the **main repo** path, although the dialog was shown in the worktree |

Trust follows the main repository root (the git common dir), not the folder hierarchy. The dialog text starts with "Accessing workspace: … Quick safety check", the default is "No, exit", and no hook fires before it.

## Other findings

- **Unix socket path**: macOS limits `sun_path` to 104 bytes. The first socket path under the scratch dir failed (`AF_UNIX path too long`). `~/Library/Application Support/<bundle id>/…` plus a long user name can get close. A `kill -9` leaves a stale socket file: connecting gives `ECONNREFUSED` at once (good for orch-hook), and the core must unlink it at start.
- **SessionEnd `reason: "other"`** for SIGHUP and SIGTERM alike; it can't tell a crash from a suspend.
- A worktree marker file named `locked` lives in the gitdir; it must not be read as "in progress".

## Proposed SPEC / ADR edits (not applied)

1. **SPEC J10 step 2, NFR-2, §6.2 (what a crash does)**: the core owns the PTYs, so a core crash **ends every agent** within ≤ 10 s (SIGHUP): `claude` kills its foreground and tracked background shells, records the interrupted tool as `Exit code 137`, and runs SessionEnd. Recovery is "reconcile, then `--resume`" for every agent, not reattaching to live processes. Say so, and that resuming within the prompt-cache TTL (1 h) is nearly free. Keeping agents alive across a core crash would need a separate per-agent PTY holder (dtach-like); not recommended for P1. PTY master fds must be close-on-exec, or a leaked master keeps a headless `claude` alive.
2. **FR-207 (kill)**: killing the agent's process group is **not enough**: Bash tool commands run in their own session (`setsid`, no tty), and `cmd &` children of those outlive `claude`. Kill = SIGTERM to `claude`'s group, to the group of every process whose **cwd is inside the agent's worktree** (`lsof -d cwd`), then SIGKILL after 5 s. Don't rely on `ORCH_AGENT_ID` in the env: macOS 27 hides the env of platform binaries (`zsh`, `bash`).
3. **SPEC J10 / NFR-2 (reconciler steps)**, add the ordered list from §A1: replay the orch-hook spool; reconcile worktrees (`creating` rows, unknown worktrees, missing ones, idempotent create = add + lock + DB); detect `MERGE_HEAD`/`REBASE_HEAD`/`rebase-*`/`CHERRY_PICK_HEAD`/`REVERT_HEAD` + unmerged count → **flag and suspend** with an Inbox item; remove a stale `index.lock` only when no git process runs in that worktree; kill processes by pgid, argv and cwd; read the transcript by session id; decide resume / suspend / flag; turn delivery is idempotent through a **turn id marker found in the transcript**; open gates come back as pending Inbox items, governor holds are requeued. Also: NFR-2 "nothing duplicated" holds for app-delivered turns, but **an interrupted non-idempotent command can leave its partial effect twice** after a resume. The resume message should ask the agent to check before redoing (it did, every time).
4. **FR-806, ADR-007 admission point 2, ADR-009 decision 4 (orch-hook when the core is down)**: "always answers before its timeout" is not enough. Add: if the socket is missing, refused, closes mid-hold, or misses orch-hook's own deadline, **PreToolUse and PermissionRequest answer JSON `deny`** ("ORCH: orchestrator unavailable…"); every other event **exits 0 at once and is spooled** to the agent folder for replay. Never `exit 2` on non-tool events (it blocks and erases the user's prompt). Only PreToolUse/PermissionRequest may wait on the core; the others use a ≤ 1 s connect timeout, or a hung core delays every prompt.
5. **FR-1201 / NFR-7 (hook binary)**: a **missing hook command fails open** (the tool runs). Install `orch-hook`/`orch-status`/`orch-mcp` at a stable path in app data (not inside the `.app` bundle, which moves and updates), check that they exist and are executable at every spawn, and add "hook binary missing" to the contract tests.
6. **FR-705, FR-806 (holds)**: while it holds a hook, the core must **watch the connection for EOF**: Esc in the pane kills orch-hook and **no hook event fires**. On EOF, close the Inbox item or governor hold as "interrupted by the user". Long holds themselves are fine (200 s held under a 600 s timeout, TUI usable, 193 MB, ~1 % CPU), so FR-705's "deny and suspend for long waits" is a memory choice, not a correctness one.
7. **§6.4 state signals and §6.2 lifecycle**:
   - **UserPromptSubmit also fires when a message is queued during a turn**, with the running turn's `prompt_id`. Same `prompt_id` → not a new turn. Queued input (the user's or the app's) is delivered with the next tool result, in the same turn.
   - **Stop with a non-empty `background_tasks` is not "idle"**: a background shell or async subagent is still running, and its completion **starts a new turn by itself** (UserPromptSubmit with a `<task-notification>` prompt).
   - SubagentStop with `agent_type: ""` and no SubagentStart (internal helpers, compaction) carries no state.
   - SessionEnd `reason` is `"other"` on crash and on suspend; use the core's own intent.
8. **ADR-007 admission point 1 and FR-410 (subagents)**: "every autonomous turn is delivered by the app" is false in 2.1.302. The Agent tool is **async by default** (`isAsync: true`), and background shells and subagents re-enter the agent with `<task-notification>` turns. The Governor must count those turns (they still pass PreToolUse admission) and may want to disallow background Bash and async subagents per role. `<usage><subagent_tokens>` in the notification and SubagentStart/Stop (`agent_id`, `agent_type`, `agent_transcript_path`) attribute subagent usage.
9. **SPEC §6.4 (API / rate-limit errors row), J9, R10**: StopFailure is verified: `error` = `"rate_limit"` or `"model_not_found"`, plus `last_assistant_message`, and no Stop follows. On a plan-limit 429 the statusline jumps to `used_percentage: 100` with the reset from the response headers (the window not reported in the headers is **absent**, not 0), and **the CLI arms its own auto-continue at the reset** ("continuing automatically at … · esc to cancel"). J9 step 3 must either let that turn happen and count it, or send Esc and reschedule through the Governor. A real limit hit is still untested.
10. **R3, FR-801, ADR-007 Measurements**: statusline `rate_limits` are **per-process caches of the last API response**. Idle `refreshInterval` re-renders the agent's own last value and doesn't see other agents' usage; after a 429 one agent can read 100 % while others read 40 %. The collector takes the **most recent sample across all agents** for each window (by sample time) and treats a missing window as "unknown", not 0. With fresh samples, all agents agreed exactly (B7).
11. **SPEC J0 step 2, ADR-003 (trust), R10**: trust is keyed by the **main repository root**. A worktree of a trusted repo doesn't prompt, wherever it lives; a worktree under a trusted `<repo>.worktrees/` folder **does**. J0: the user trusts the project root once; drop the worktree-root pane. After a repo move the dialog comes back once. The app detects it on the PTY ("Quick safety check"), shows that pane to the user, and never answers it itself.
12. **ADR-003, FR-105 (moves)**: create worktrees with **`worktree.useRelativePaths=true`** (git ≥ 2.48; it sets `extensions.relativeWorktrees`, so older git can't open the repo: note it in the supported-git check) so a joint move of the repo and its sibling folder needs no repair. Otherwise the reconciler runs `git -C <repo> worktree repair <every known worktree path>` (plain `repair` from the main checkout can't find moved worktrees) and re-locks. Locks keep moved worktrees out of `prune`.
13. **FR-209, ADR-005 (transcripts)**: look transcripts up **by session id** across `~/.claude/projects/*/`: after a move, `--resume <id>` keeps appending to the transcript in the old project folder.
14. **NFR-3 / ADR-009 decision 5 (socket path)**: keep the socket path under macOS's 104-byte limit (check the length at start) and unlink a stale socket file at start.
15. **NFR-1b, FR-203 (numbers)**: 189–206 MB phys_footprint per agent with `--strict-mcp-config` (idle or in a sleeping tool), so 3 agents ≈ 0.6 GB, on a machine with 7–8 GB of swap in use. The default cap of 3 holds.
16. **R10**: mark as answered: long holds while typing (B4), SubagentStart/Stop and PreCompact payloads (B5), StopFailure (B6, simulated 429), several agents (B7), trust in a worktree (B8). Still open: a real plan-limit hit, and the CLI's own auto-continue at the reset.

## Follow-ups not covered here

- A real plan-limit hit, and what the CLI's auto-continue at the reset does to hooks and the statusline.
- Whether `--disallowed-tools` or a setting can turn off async subagents and `run_in_background` per role.
- App-level crash points other than worktree creation (mid gate decision, mid PR creation) once those flows exist.
- Sandbox interaction: with the ADR-009 sandbox on, whether detached `&` children are confined or killed differently.
