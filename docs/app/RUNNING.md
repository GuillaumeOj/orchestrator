# Running Orchestrator day to day

The P1 app: real Claude Code agents in panes, projects and git, the Inbox, and the Week planner
with the Governor in **shadow mode** for the first week (it logs what it would do and holds
nothing). This page is for using it, not developing it ([crates/orch-app](../../crates/orch-app/README.md)).

## Build and install

```bash
make install
```

`scripts/app.sh install` builds the helpers and the app in release, bundles them, and copies
`Orchestrator.app` to `~/Applications` (refused while the app runs: quit it with ⌘Q first).
`make app` only builds (`target/release/bundle/macos/Orchestrator.app`).

Needs: Rust (rustup), bun, Xcode command line tools, git ≥ 2.48, and FiraCode Nerd Font
Mono in `~/Library/Fonts` (the build bundles four of its weights into the app; `ORCH_FONT_DIR`
points elsewhere). `gh` (logged in) for PRs and CI. The app is not signed or notarized: it is
built on this Mac, so Gatekeeper does not quarantine it.

## First run (J0)

Open `~/Applications/Orchestrator.app`. The **Set up Orchestrator** pane opens until you click
Done (palette: "Set up Orchestrator" reopens it).

1. **Claude Code.** The app runs `claude --version` and `claude auth status` (it never writes to
   `~/.claude` or `~/.claude.json`). Supported: 2.1.302 (newer versions run with a warning: run the
   contract tests). Not installed, too old, not logged in, or logged in with an API key instead
   of Pro/Max: a red "Read-only" banner says what to fix, and agents cannot start until you fix it
   and restart the app. The app never sets `ANTHROPIC_API_KEY` and warns if it inherits one.
2. **Projects.** Add project (⌘N or `+` in the sidebar): a folder (a worktree registers its main
   checkout) or a git URL to clone (into `~/dev` by default, with your own credentials). Projects
   are kept in the store; a background `git fetch` runs every 15 min for projects active in the
   last 7 days (paused on battery below 20 % and offline).
3. **Trust each project.** Claude Code asks once per repository, before any hook runs, whether
   you trust the folder. "Trust…" opens plain `claude` in the project root in a pane: move down to
   **Yes, I trust this folder**, press ⏎, then type `/exit` and close the pane (⌘W). Trust covers
   every worktree of the repository. The app reads the answer from `~/.claude.json` and never
   answers the dialog itself. If you skip this, the first agent's pane shows the dialog instead.
4. **Terminal look and editor.** kitty.conf is imported read-only (default
   `~/.config/kitty/kitty.conf`; another path can be imported). The editor command defaults to
   kitty's `editor`, else `nvim +{line} {file}`.
5. **First weekly plan.** Working days and hours, the reserve for your own sessions, and one
   weighted allocation per project (an even split is proposed). It applies to the current plan
   week, which follows Claude's weekly reset; this week's usage is kept. Edit it later from the
   Week planner (`e`).

## A normal day

- ⌘T new session (a worktree `orch/solo/<name>` in `<repo>.worktrees/`), ⌘; o Overview,
  ⌘; i Inbox, ⌘; g Week planner, ⌘K palette (every action), ⌘P jump.
- Clicking a project in the sidebar opens its pane: branch, ahead/behind, changes, stash, last
  commit, Fetch (`f`), worktrees with their state and Remove (unmerged work asks separately), PRs
  and CI (`p`, via `gh`), and Remove project (`x`, J11: agents stopped and archived, merged
  worktrees removed, unmerged ones kept unless you tick them, then archive or delete history).
- Quitting (⌘Q) suspends the agents; the next start shows them Suspended (⏎ resumes). After a
  crash the reconciler resumes the agents that were working.

## The shadow week

For 7 days from the first start the Governor only logs: the Week planner shows "Shadow mode ·
day n of 7", its log lists what it would have done (including earlier runs of the app), and the
title bar shows the pressure state with "· shadow". The "actual" line and the forecast cover the
whole plan week across restarts (the store keeps a point every 5 minutes while the app runs).
After the week, "Shadow week done · Enforce…" switches it on; "Shadow mode…" switches it back.
P1's exit check: the forecast within ±10 pp of the observed weekly % at the reset.

## Where data lives

`~/Library/Application Support/Orchestrator` (`ORCH_HOME` overrides it):

| Path | What |
|---|---|
| `state/orch.db` (+ `-wal`, `-shm`, `orch.blobs/`) | SQLite store: projects, agents, worktree rows, Inbox, rules, usage samples, plans, forecasts, governor actions, settings (`kv`, keys `ui.*`), audit log, transcript index |
| `bin/` | `orch-hook`, `orch-status`, `orch-mcp`, copied from the app bundle at every start when they changed (a stable path outside the `.app`) |
| `agents/<id>/` | Per-agent settings layer, MCP config, git config, hook spool |
| `logs/orchestrator.log` (+ `.1`, the previous run) | The app's log when started from the Finder |
| `logs/memory.log` (+ `.1`) | One JSON line a minute: core and WebKit helper footprints |
| `nvim/` | nvim server sockets, one per project |
| `core.sock` | The bus socket (hooks, statusline, `orchd … --home`) |

Worktrees live next to each repository (`<repo>.worktrees/`). Claude Code's own transcripts stay
in `~/.claude/projects` (read only). A `ui/settings.json` from the first app version is imported
once and renamed `settings.json.migrated`.

## Logs and checks

```bash
tail -f ~/Library/Application\ Support/Orchestrator/logs/orchestrator.log
```

```bash
tail -n 5 ~/Library/Application\ Support/Orchestrator/logs/memory.log
```

```bash
cargo run -q -p orchd -- usage --home ~/Library/Application\ Support/Orchestrator
```

`orchd state|usage|governor|events|audit --home …` read the running app through its socket.
Started from a terminal (`~/Applications/Orchestrator.app/Contents/MacOS/orchestrator`), the app
logs to the terminal instead.

**Memory watch.** Once a WebContent runaway reached 2.8 GB after resizing the window with agent,
nvim and shell panes. Every minute the app logs its WebKit helpers' footprints to `memory.log`; a
helper above 600 MB (`ORCH_WEBKIT_WARN_MB`) puts a red "⚠ WebContent … MB" in the status bar and a
line in the log. If you see it: note what you just did, keep the log, and quit the app (⌘Q keeps
the agents' sessions; or `kill` the WebContent process named in Activity Monitor).

## Reset

Quit the app first.

- Settings and layout only: `sqlite3 ~/Library/Application\ Support/Orchestrator/state/orch.db "DELETE FROM kv WHERE key LIKE 'ui.%'"` (the first-run pane comes back, and the shadow week starts again).
- Everything the app knows (projects, history, agents' records): move the folder away,
  `mv ~/Library/Application\ Support/Orchestrator ~/Orchestrator-old`. Worktrees and branches in
  your repositories are not touched; remove them from the project pane before, or with
  `git worktree remove` after.
- WebKit caches: `~/Library/Caches/dev.orchestrator.app`, `~/Library/WebKit/dev.orchestrator.app`.

## Known limits (P1)

- Solo sessions only (teams, gates and the PR gate UI come in P2); no diff pane yet (FR-107 is in
  the core); pause per agent is Pause all.
- The Governor's queue is not restored after a restart: turns waiting in it are asked again.
  Changing the plan or switching enforce is refused while turns are queued.
- Plan editing covers days, hours, reserve, weights and tiers; holidays, off-hours allowance and
  hard caps are not in the form yet.
- The bundle is unsigned; the helpers must stay executable in `bin/` (checked at every spawn).
- ⌘-chords against the menu, dead keys and IME were not checked with physical keys (R9).
- The "actual" line has one point every 5 minutes for earlier runs, every minute for this one.
- `gh` must be on the login shell's PATH (the app takes PATH from your login shell).
