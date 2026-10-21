# orchd

A headless core for development: runs the bus and the agent runtime
([agent-runtime](../agent-runtime/README.md)) and drives agents from the command line. No UI.

```bash
cargo build
target/debug/orchd install-helpers           # copies orch-hook/orch-status to <home>/bin
target/debug/orchd serve &                   # foreground core; Ctrl-C suspends every agent
target/debug/orchd start a1 --repo ~/dev/myproj --model haiku --strict-mcp --message "Summarize README.md"
target/debug/orchd state                     # agents: state, pid, memory (phys_footprint), context, cost, turns
target/debug/orchd usage                     # 5h / weekly (most recent across agents) + Governor W/F
target/debug/orchd governor                  # plan, snapshot, queue, actions, admission decisions
target/debug/orchd send a1 "Now run the tests"
target/debug/orchd attach a1                 # this terminal ⇄ the agent's PTY; Ctrl-] detaches
target/debug/orchd suspend a1 && target/debug/orchd resume a1 --message "Continue"
target/debug/orchd kill a1
target/debug/orchd stop
```

- Home: `--home DIR`, else `$ORCH_HOME`, else `~/Library/Application Support/Orchestrator`. The socket
  is `<home>/core.sock` (checked < 104 bytes, 0600, stale file removed at start).
- `serve` runs the J10 reconciler first (`--no-recover` to skip), refuses to start if another core
  answers, and runs the Governor in **shadow mode** unless `--enforce`. `--cap N` (live cap base),
  `--hold-secs N` (PreToolUse hold), `--otel URL` (OTLP endpoint for agents; no receiver is bundled).
- `worktree <repo> solo <slug> | integration <feature> | team <feature> <role> <n>` creates a worktree
  without an agent. `start` creates `orch/solo/<slug>` (default slug = agent id).
- `start` options: `--slug --base --model --effort --mode --prompt --strict-mcp --allow-host H…
  --allow RULE… --tier --allocation --message`. `--allow Bash` skips Bash prompts inside the sandbox.
- `reconcile` prints a dry run of the reconciler (core stopped). `pause-all` / `resume-all` drive the
  Governor's Pause all (FR-811).
- A new repo shows Claude Code's trust dialog on the first start: `state` flags `trust_dialog`, an
  Inbox item appears, and nothing is delivered until you `attach` and answer it yourself.
- Stopping the core (`stop`, Ctrl-C) suspends agents. Killing it hard ends them anyway (SIGHUP on
  the PTYs); the next `serve` reconciles and resumes the ones that were working.
