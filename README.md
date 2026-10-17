# Orchestrator

A personal, local-first desktop app to run software projects with AI agents. Manage local projects and their git/remote state, launch solo sessions or teams of specialist agents (lead, backend, frontend, UI, UX) in isolated git worktrees, chat with them like in Claude Code, keep your own terminal and nvim, and **pace all agent work so your Claude Max plan lasts the whole week**.

Status: specification phase, no code yet.

- [Specification](docs/SPEC.md)
- Architecture decisions:
  - [ADR-001 Desktop shell: Tauri + Rust core + Node sidecar (low memory)](docs/adr/001-desktop-shell.md)
  - [ADR-002 Claude Agent SDK behind a Provider adapter](docs/adr/002-agent-runtime-provider-adapter.md)
  - [ADR-003 Worktree per agent + integration branch](docs/adr/003-worktree-isolation.md)
  - [ADR-004 Pluggable task backends](docs/adr/004-pluggable-task-backends.md)
  - [ADR-005 Local persistence](docs/adr/005-local-persistence.md)
  - [ADR-006 Terminal & editor](docs/adr/006-terminal-and-editor.md)
  - [ADR-007 Usage pacing governor](docs/adr/007-usage-pacing-governor.md)
  - [ADR-008 UI source of truth: Penpot file](docs/adr/008-ui-design-source-of-truth-penpot.md)
