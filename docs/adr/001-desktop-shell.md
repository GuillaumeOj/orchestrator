# ADR-001 — Desktop shell: Tauri 2 + Rust core + one Node sidecar

- Status: Proposed · Date: 2026-10-17

## Context
- Orchestrator spawns many agent sessions, PTYs and git commands, and must host the Claude Agent SDK, which is a TypeScript/Node library.
- **The target laptop has limited resources: memory use must be as low as possible.** This is a primary constraint, ahead of developer convenience.
- Most of the memory goes to the **agent processes**, not the UI. Each Claude Agent SDK session runs its own Claude Code process. The architecture must reduce both the shell's footprint and the number of live agent processes.

## Decision
1. **Tauri 2** shell: it uses the system webview (WKWebView on macOS), so no Chromium is bundled. UI in a lightweight framework (SolidJS or Svelte preferred over React for its smaller runtime; final pick in the P0 spike).
2. **Rust core** for everything that is not the SDK: project and git service (git CLI, gitoxide for status), PTYs (`portable-pty`), SQLite (`rusqlite`), file watching, Governor, Inbox, task backends, internal MCP server.
3. **A single Node sidecar** (Node, or Bun if compatible) that hosts **all** Agent SDK sessions and talks to the core over a local socket (JSON-RPC). It is never one sidecar per session.
4. **Memory-aware session management** (also in SPEC NFR-1b):
   - **Idle sessions are suspended**: their agent process is stopped after N minutes idle (configurable) and **resumed on demand** from the persisted provider session id. Only *working* agents hold a process.
   - The Governor enforces a **max number of live agent processes**, alongside the usage budget. Queued work waits instead of spawning.
   - Terminal scrollback is capped and older output goes to disk. Chat transcripts are virtualized and paged from SQLite.
   - Background git fetches and file watchers are throttled and limited to active projects.
   - The app shows its own memory use (shell + sidecar + each agent process) in the Monitor.

## Alternatives
- **Electron + TypeScript**: one language and the SDK in-process, but it bundles Chromium and a Node main process, a much higher fixed memory cost. Rejected because of the memory constraint.
- **Native Swift**: lowest footprint, but no SDK, no xterm.js equivalent, macOS only, much more work.
- **Local web app (server + browser tab)**: low footprint if the browser is already open, but weaker OS integration (notifications, keychain, kitty launch) and less reliable lifecycle.

## Consequences
- Lower fixed memory use; the dominant cost (agent processes) is bounded by suspension and the process cap.
- Two languages (Rust + TypeScript) and an IPC boundary between core and sidecar. The `Provider` interface (ADR-002) sits on that boundary.
- Rendering must be validated in WKWebView (xterm.js WebGL renderer, fonts, ligatures) during the P0 spike.
