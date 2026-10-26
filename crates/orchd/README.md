# orchd

A headless core for development: runs the bus, the agent runtime
([agent-runtime](../agent-runtime/README.md)), the SQLite store ([orch-store](../orch-store/README.md)),
the git service ([orch-git](../orch-git/README.md)) and the local OTLP receiver, and drives them from
the command line. No UI.

```bash
cargo build
target/debug/orchd install-helpers           # copies orch-hook, orch-status, orch-mcp to <home>/bin
target/debug/orchd serve &                   # foreground core; Ctrl-C suspends every agent
target/debug/orchd project add ~/dev/myproj  # FR-101 (clone: project clone <url> <parent>)
target/debug/orchd start a1 --repo ~/dev/myproj --model haiku --strict-mcp --message "Summarize README.md"
target/debug/orchd state                     # agents: state, pid, memory (phys_footprint), context, cost, turns
target/debug/orchd inbox                     # permission prompts, gates, alerts
target/debug/orchd decide 3 approve          # or reject | always [--scope project|role|agent] | reply --message … | dismiss
target/debug/orchd projects                  # J1: branch, ahead/behind, dirty, live agents
target/debug/orchd project show myproj --prs # worktrees (orphans flagged), branches, PRs + CI
target/debug/orchd diff ~/dev/myproj.worktrees/solo/a1
target/debug/orchd audit a1                  # commits, permission decisions, rule changes
target/debug/orchd search "flaky test"       # FTS over indexed transcripts
target/debug/orchd usage | governor          # 5h / weekly + Governor W/F, plan, queue, actions
target/debug/orchd suspend a1 && target/debug/orchd resume a1 --message "Continue"
target/debug/orchd stop
```

- Home: `--home DIR`, else `$ORCH_HOME`, else `~/Library/Application Support/Orchestrator`. The socket
  is `<home>/core.sock` (checked < 104 bytes, 0600, stale file removed at start); the database is
  `<home>/state/orch.db` (a `state.json` from earlier builds is imported once).
- `serve` runs the J10 reconciler first (`--no-recover` to skip), refuses to start if another core
  answers, and runs the Governor in **shadow mode** unless `--enforce`. Options: `--cap N` (live cap
  base, lowered under memory pressure), `--hold-secs N` (PreToolUse hold), `--permission-hold-secs N`
  (Inbox hold for permission prompts; 0 = straight to the pane), `--idle-min N` (idle auto-suspend,
  0 = off, default 10), `--fetch-min N` (background fetch interval, default 15), `--otel URL`
  (external receiver) or `--no-otel` (default: the bundled receiver on 127.0.0.1, ephemeral port).
- Agents commit with `mcp__orch__commit` (orch-mcp); `orchd commit <agent> <message> [--path P]`
  does the same from the CLI. `rule add role solo mcp__orch__commit ask` makes every commit wait
  for an Inbox approval.
- `worktree <repo> solo <slug> | integration <feature> | team <feature> <role> <n>` creates a worktree
  without an agent; `worktree-rm <project> <path> [--delete-branch] [--discard]` removes one
  (unmerged work needs `--discard`). `project remove-plan` / `project remove --history delete|archive`
  is J11; `project relocate <id> <new-path>` repairs worktrees after a repo move.
- `reconcile` prints a dry run of the reconciler (core stopped). `pause-all` / `resume-all` drive the
  Governor's Pause all (FR-811).
- A new repo shows Claude Code's trust dialog on the first start: `state` flags `trust_dialog`, an
  Inbox item appears, and nothing is delivered until you `attach` and answer it yourself.
- Stopping the core (`stop`, Ctrl-C, SIGTERM) suspends agents. Killing it hard ends them anyway
  (SIGHUP on the PTYs); the next `serve` reconciles from the database and resumes the ones that
  were working (`tests/crash_restart.rs`).
