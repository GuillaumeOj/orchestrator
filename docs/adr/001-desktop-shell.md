# ADR-001 — Desktop shell: Tauri 2 + Rust core, no Node runtime

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (review: no SDK, so no Node sidecar)

## Context
- **The target laptop has limited RAM**, so memory use is a primary constraint.
- Agents are real `claude` CLI processes (ADR-002). They use most of the memory; the app shell should add as little as possible on top.
- With no Agent SDK there is no reason to ship a Node runtime.

## Decision
1. **Tauri 2** shell with the system webview (WKWebView) and no bundled Chromium. The UI uses **Svelte 5** (decided in the P0 terminal spike: small compiled runtime, enough for an xterm-heavy UI; SolidJS showed no advantage).
2. **Rust core** for everything: projects and git (git CLI; gitoxide for fast status), PTYs (`portable-pty`), SQLite (`rusqlite`, WAL), the Governor, Inbox, task backends, an OTLP receiver, and the local bus (a Unix socket).
3. **Helper binaries** written in Rust and shipped with the app: `orch-hook`, `orch-status` and `orch-mcp`. Claude Code launches them (hooks, statusline, stdio MCP), and they talk to the core over the bus.
4. **PTY output** reaches the UI through a **pull command** (`pty_read`) that returns all buffered bytes as a raw IPC response. The core buffers each pane up to a high watermark, which gives backpressure. P0 measured Tauri 2 Channels at 2–5 s UI stalls with 6 busy panes (2.2 M IPC messages), and one lost message stalled 2 panes for good. Channels are kept for low-rate events only.
5. **Memory management**: a live-agent cap, suspend/resume of idle agents (process exits and is resumed by session id), capped scrollback, hidden panes not rendering, and git fetches limited to active projects. The app shows its own memory, including the WebContent/GPU/Network helper processes. Measured in P0: about 75 MB idle with no pane. The first WebGL terminal adds about 110 MB in the GPU helper, so terminals use the DOM renderer by default (SPEC FR-1005).

## Alternatives
- **Electron**: bundles Chromium plus a Node main process, a much higher fixed memory cost. Rejected.
- **Native Swift**: lowest footprint, but no xterm.js equivalent, macOS only, much more UI work.
- **Local web app**: weaker OS integration (notifications, Keychain, power assertions).

## Consequences
- One language for the core and the helpers, no sidecar, and no single process whose crash takes all agents down. Each `claude` runs in its own process group.
- Rendering must be validated in WKWebView (xterm.js WebGL, fonts, ligatures, WebGL context limit) in P0.
