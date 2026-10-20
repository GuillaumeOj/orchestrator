# P0 spike: crash/restart reconciliation

Throwaway harness used for [docs/spikes/p0-recovery.md](../../docs/spikes/p0-recovery.md). This is not product code.

| File | Role |
|---|---|
| `core.py` | Stand-in for the core. `setup` makes a scratch repo `ROOT/proj/demo` with the sibling `demo.worktrees/`. `serve` runs a Unix socket (0600), a SQLite DB (WAL) and the agents' PTYs **inside the core process**, the way the Tauri core would. Each agent gets a worktree (`orch/solo/<id>`, `git worktree lock`ed), a per-agent `--settings` file (all hooks → `orch_hook.py` with `timeout: 600`, statusline → `orch_status.py` with `refreshInterval: 5`), a from-scratch env, `--session-id`, `--model haiku`, `--strict-mcp-config`. `serve --recover` runs the reconciler first and applies it. `reconcile` is a dry run. `ctl` sends control commands (spawn, deliver, type, release, suspend, screen, footprint, state). |
| `orch_hook.py` | Stand-in for `orch-hook`. Forwards the payload to the core and prints the answer. If the core can't be reached it denies PreToolUse/PermissionRequest (fail closed) and spools the event to the agent folder. `ORCH_HOOK_FALLBACK=open|exit1|exit2` reproduces the other behaviors tested in A2. |
| `orch_status.py` | Stand-in for `orch-status`. Logs the statusline JSON and forwards it. |
| `mock_429.py` | Local HTTP stand-in for the API that answers `/v1/messages` with a plan rate-limit 429 (B6). It never reads or logs request headers. |
| `scrub_samples.py` | Extracts and scrubs the sample payloads in `samples/`. |
| `samples/` | Hook payloads (subagents, compaction, StopFailure, queued prompts…), statusline, orch-hook spool, one reconciler report. |

Policy for PreToolUse lives in `ROOT/app/policy.json`, re-read on every call:

```json
{"pre": [{"match": "git status", "action": "gate", "hold_s": 3000},
         {"match": "gov-me", "action": "gov", "hold_s": 600, "on_timeout": "deny"}],
 "perm": "allow"}
```

Example:

```bash
./core.py setup /path/to/scratch/r
./core.py serve /path/to/scratch/r &
./core.py ctl /path/to/scratch/r '{"cmd":"spawn","agent":"a1"}'
./core.py ctl /path/to/scratch/r '{"cmd":"deliver","agent":"a1","body":"Reply OK."}'
kill -9 "$(cat /path/to/scratch/r/app/core.pid)"
./core.py serve /path/to/scratch/r --recover &
```

The socket lives at `/tmp/orchspike-<hash>.sock`: a socket path under the scratch dir was longer than macOS's 104-byte `sun_path` limit.
