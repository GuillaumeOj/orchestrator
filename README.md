# Orchestrator

A personal, local-first macOS app to run software projects with AI agents, built around the official **Claude Code CLI**. Manage local projects and their git/remote state, launch solo sessions or canvas-designed teams of specialist agents (lead, backend, frontend, UI, UX) in isolated git worktrees, work with each agent in its real Claude Code session next to a kitty-like terminal and your own nvim, and **pace all agent work so your Claude Max plan lasts the whole week**.

Status: **P1 app in daily use** (SPEC v0.9). It runs real Claude Code agents in panes, projects and git, the Inbox and the Week planner; the Governor runs in **shadow mode** for now (it logs what it would do and holds nothing). Screenshots next to the Penpot design are in [docs/app](docs/app/README.md).

## Build and run

macOS only. Needs Rust (rustup), bun, Xcode command line tools, git ≥ 2.48, the Claude Code CLI logged in with a Pro/Max plan, and `gh` for PRs. Details and first run: [docs/app/RUNNING.md](docs/app/RUNNING.md).

```bash
make install   # build Orchestrator.app (release) and copy it to ~/Applications
make test      # cargo test + bun run test in ui/
make clippy
make fmt       # cargo fmt + biome (format, imports, safe fixes) in ui/
make lint      # what CI checks: fmt, clippy, biome, svelte-check
```

## Layout

- [`crates/governor`](crates/governor/README.md): the usage-pacing Governor, a pure control law with no I/O; `crates/pacing-sim` is its week simulator.
- [`crates/orch-protocol`](crates/orch-protocol/README.md): versioned messages and socket helpers for the local bus.
- [`crates/orch-hook`](crates/orch-hook/README.md), [`crates/orch-status`](crates/orch-status/README.md): the hook and statusLine commands Claude Code runs for each agent.
- [`crates/orch-mcp`](crates/orch-mcp/README.md): the orchestrator's MCP server (commit, request_pr outside the sandbox).
- [`crates/orch-store`](crates/orch-store/README.md): SQLite persistence (WAL, migrations, events, transcript search).
- [`crates/orch-git`](crates/orch-git/README.md): project registry and git service (status, fetch, worktrees, diffs, `gh`).
- [`crates/agent-runtime`](crates/agent-runtime/README.md): the headless agent runtime (spawn `claude` in PTYs, observe, steer, recover).
- [`crates/orchd`](crates/orchd/README.md): a headless dev core driven from the command line.
- [`crates/orch-app`](crates/orch-app/README.md): the desktop app (Tauri 2), with its Svelte 5 UI in [`ui/`](ui).

## Docs

- [Specification](docs/SPEC.md)
- [Spec review 2026-10-17](docs/reviews/2026-10-17-spec-review.md)
- Architecture decisions:
  - [ADR-001 Desktop shell: Tauri 2 + Rust core, no Node runtime](docs/adr/001-desktop-shell.md)
  - [ADR-002 Agent runtime: the official Claude Code CLI](docs/adr/002-agent-runtime-provider-adapter.md)
  - [ADR-003 Worktree per agent in a sibling folder](docs/adr/003-worktree-isolation.md)
  - [ADR-004 Pluggable task backends](docs/adr/004-pluggable-task-backends.md)
  - [ADR-005 Local persistence](docs/adr/005-local-persistence.md)
  - [ADR-006 Terminal & editor](docs/adr/006-terminal-and-editor.md)
  - [ADR-007 Usage pacing: Governor control law](docs/adr/007-usage-pacing-governor.md)
  - [ADR-008 UI source of truth: Penpot file](docs/adr/008-ui-design-source-of-truth-penpot.md)
  - [ADR-009 Security: sandbox, credentials, gates](docs/adr/009-security-sandbox-and-gates.md)
- [Design notes](docs/design/README.md) (Penpot source of truth, tokens)
- P0 spikes: [Claude Code CLI](docs/spikes/p0-claude-cli.md), [terminal](docs/spikes/p0-terminal.md), [recovery](docs/spikes/p0-recovery.md)
