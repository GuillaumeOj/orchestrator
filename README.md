# Orchestrator

A personal, local-first macOS app to run software projects with AI agents, built around the official **Claude Code CLI**. Manage local projects and their git/remote state, launch solo sessions or canvas-designed teams of specialist agents (lead, backend, frontend, UI, UX) in isolated git worktrees, work with each agent in its real Claude Code session next to a kitty-like terminal and your own nvim, and **pace all agent work so your Claude Max plan lasts the whole week**.

Status: specification phase (SPEC v0.2), no code yet.

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
