# ADR-001 — Desktop shell: Tauri 2 + Rust core, no Node runtime

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (review: no SDK, so no Node sidecar)

## Context
- **The target laptop has limited RAM**, so memory use is a primary constraint.
- Agents are real `claude` CLI processes (ADR-002). They use most of the memory; the app shell should add as little as possible on top.
- With no Agent SDK there is no reason to ship a Node runtime.

## Decision
1. **Tauri 2** shell with the system webview (WKWebView) and no bundled Chromium. The UI uses SolidJS or Svelte (smaller runtime than React); the final pick is made in P0.
2. **Rust core** for everything: projects and git (git CLI; gitoxide for fast status), PTYs (`portable-pty`), SQLite (`rusqlite`, WAL), the Governor, Inbox, task backends, an OTLP receiver, and the local bus (a Unix socket).
3. **Helper binaries** written in Rust and shipped with the app: `orch-hook`, `orch-status` and `orch-mcp`. Claude Code launches them (hooks, statusline, stdio MCP), and they talk to the core over the bus.
4. **PTY output** reaches the UI through Tauri 2 Channels with binary payloads, not JSON events.
5. **Memory management**: a live-agent cap, suspend/resume of idle agents (process exits and is resumed by session id), capped scrollback, hidden panes not rendering, and git fetches limited to active projects. The app shows its own memory, including the WebContent/GPU/Network helper processes.

## Alternatives
- **Electron**: bundles Chromium plus a Node main process, a much higher fixed memory cost. Rejected.
- **Native Swift**: lowest footprint, but no xterm.js equivalent, macOS only, much more UI work.
- **Local web app**: weaker OS integration (notifications, Keychain, power assertions).

## Consequences
- One language for the core and the helpers, no sidecar, and no single process whose crash takes all agents down. Each `claude` runs in its own process group.
- Rendering must be validated in WKWebView (xterm.js WebGL, fonts, ligatures, WebGL context limit) in P0.
