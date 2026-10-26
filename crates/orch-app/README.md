# orch-app

The Orchestrator desktop app: a **Tauri 2** shell whose Rust side embeds the
[agent runtime](../agent-runtime/README.md) and its [Governor](../governor/README.md), with a
**Svelte 5** UI in [`ui/`](../../ui). SPEC v0.6 §9, §4.2, §4.9–4.11, FR-1001–1104, FR-1204,
NFR-1/1b; ADR-001, ADR-006.

## Run it

```bash
cargo build -p orch-hook -p orch-status -p orch-mcp   # the helpers Claude Code launches
cd ui && npm install
npm run tauri dev                                # Vite on :1430 + the app (debug)
npm run tauri build -- --no-bundle               # release binary: target/release/orchestrator
```

At start the app copies `orch-hook`/`orch-status`/`orch-mcp` from next to its binary into `<home>/bin` (as
`orchd install-helpers` does). Environment:

| Variable | Effect |
|---|---|
| `ORCH_HOME` | App data (state, socket, agents, settings). Default `~/Library/Application Support/Orchestrator`, as `orchd`. |
| `ORCH_PROJECTS` | Colon-separated folders to register at start (the stub registry keeps nothing; see below). |
| `ORCH_CLAUDE` | `claude` binary (default `~/.local/bin/claude`). |
| `ORCH_KITTY_CONF` | kitty.conf to import, read only (default `~/.config/kitty/kitty.conf`). |
| `ORCH_NO_RECOVER` | Skip the J10 reconciler at start. |
| `ORCH_ENFORCE` | Enforce the Governor (default: shadow mode, FR-812). |
| `ORCH_DEV_CTL`, `ORCH_DEV_LOG` | Development harness, off by default ([`devctl.rs`](src/devctl.rs)): JSON lines on that socket run UI commands or snippets (`{"cmd":"run","id":"inbox"}`, `{"cmd":"eval","js":"…"}`); results go to the log. Used for the screenshots and measurements in [docs/app](../../docs/app/README.md). |

The `.app` bundle does not contain the helpers yet: copy `orch-hook`/`orch-status` into
`<home>/bin` (or run `orchd install-helpers`) before starting agents from a bundle. `open -n --env
ORCH_HOME=… Orchestrator.app` passes the variables to a bundle.

The bus socket is `<home>/core.sock`, so `orchd state|usage|governor|send|attach --home <home>`
work against the running app. `orchd stop` does not quit it (quit the app to stop its core).

## Embed, not talk to orchd: why

The Tauri process **embeds** `agent_runtime::Runtime` and serves the bus socket itself, instead
of being a client of a separate `orchd`:

- **Memory (NFR-1b, the primary constraint).** One process instead of two; nothing to keep
  running when the window is closed.
- **The pull transport stays one hop.** `agent_read` drains the agent's PTY buffer in-process and
  returns it as one raw IPC response (ADR-001 §4). Through orchd every byte would cross the Unix
  socket a second time, and orchd's `attach` is a push stream, the shape P0 measured stalling.
- **ADR-001 already says** "Rust core for everything" inside the Tauri app; orchd was the headless
  stand-in until the app existed. It stays useful for development, and talks to the app's socket.
- The cost is the one ADR-001 accepts: a crash of the app ends its agents (SIGHUP), and the next
  start reconciles and resumes them (J10, `Runtime::recover`).

The app refuses to start its core when another core answers on the same socket (an `orchd serve`):
the UI then runs without agents and says why in a banner.

## Backend services that are stubs

Another session builds the SQLite store, the project and git registry and the OTLP receiver as
new crates. The app codes against two small traits in [`services.rs`](src/services.rs) and ships
stubs:

| Trait | Stub | Real implementation |
|---|---|---|
| `ProjectRegistry` (list, add, remove, git summary) | `InMemoryProjects`: in memory, seeded from `ORCH_PROJECTS` and from the repos of known agents; git state from one `git status --porcelain=v2 --branch` (no fetch, no PRs) | the project/git registry crate |
| `SettingsStore` (saved layout, theme, the project each session was started from) | `JsonFileSettings`: `<home>/ui/settings.json` | a table in the SQLite store |

The OTLP receiver needs no trait yet: the runtime only sets the OTEL env when an endpoint is
configured, and the Governor is fed by the statusline. The Week planner's "actual" line is the
history the app samples every minute while it runs (in memory), so it starts at app start.

## What is in the crate

| Module | What it does | SPEC |
|---|---|---|
| `core` | Embeds the runtime; bus server (hooks, statusline, `orchd` control); loops (pump 200 ms, tick 30 s, history 60 s); the `app_state` view; agent, terminal, editor, Inbox and Governor actions | §6, ADR-001 |
| `permissions` | `PermissionRequest` hooks held for the Inbox: allow, deny (with a message), or "answer in the pane" (no decision: the CLI shows its dialog). Esc in the pane (EOF) closes the item; no answer before orch-hook's deadline falls back to the pane | FR-705, J5 |
| `inbox` | Attention ladder: held permissions, agents waiting for you, trust dialogs (1); errors and runtime alerts (2); the Governor's pending ask (3); finished turns (4, digest). Badge = levels 1–3 | FR-707, §9.6 |
| `planner` | Week planner view: ideal curve, actual (sampled), forecast at the current burn, an estimate "with governor", working blocks, allocations, queue, log with undo, next action, pending ask | FR-802–804, FR-809, J8 |
| `terms` | Shell and editor PTYs (login shell, minimal env), pull transport with a 2 MB high watermark | FR-1001, ADR-006 |
| `editor` | Editor command template (`nvim +{line} {file}`, or kitty.conf's `editor`); one nvim server socket per project, reused through `--remote-send` | FR-1101/1102 |
| `kitty` | kitty.conf importer from the P0 spike, plus the four kitty-look fixes (resolve the font as kitty does, CoreText cell height, kitty's Nerd Font symbols) | FR-1002 |
| `fonts` | Font files through CoreText for the webview (WKWebView hides user fonts) | ADR-006 |
| `memory` | phys_footprint of the app and the WebKit helpers it is responsible for, shells, agents | FR-904, NFR-1b |
| `menu` | The app's own macOS menu: ⌘W closes a pane (Close Window is ⇧⌘W), the app's chords listed | §9.7 |
| `commands` | Tauri commands, thin wrappers over `core` | |

Additions to existing crates (wiring only): `Runtime::live`, `Runtime::with_governor`,
`Runtime::governor_act` and `Live::user_input` in `agent-runtime`.

## UI (`ui/`)

- `src/lib`: the app store (`app.svelte.js`: polling, panes, focus, commands), the keyboard
  contract (`keyboard.js`), the tiling layout (`layout.js`), Inbox triage (`inbox.js`), the §9.5
  vocabulary (`status.js`), the terminal pane (`term.js`: pull loop, kitty look, path links),
  fonts through CoreText (`fonts.js`), generated `tokens.js`.
- `src/components`: shell parts (title bar with the pacing indicator, sidebar, icon rail, tiling
  workspace, pane frame, inspector, status bar, palette and jump, dialogs, toasts, banners) and the
  base components (status badge, agent chip, pressure pill, key hint).
- `src/panes`: Agent, Shell/Editor, Overview, Inbox, Week planner.
- **Tokens** (`scripts/tokens.mjs`): `docs/design/tokens.json` → `src/styles/tokens.css` (Dark,
  Light, system) and `src/lib/tokens.js`; semantic tokens only. `npm run tokens:check` (also run
  by `npm run build` and the vitest suite) fails when the generated files drift from tokens.json,
  when UI code uses a raw color, or when it names an unknown token. A small script instead of
  Style Dictionary: no dependency, ~150 lines, same output shape.
- **Keyboard** (§9.7): one capture-phase handler: ⌘-chords and the leader `⌘;` never reach
  xterm.js; Shift+Enter → `ESC CR` in agent panes; kitty `map … send_text` remaps; everything else
  goes to the focused element. The app's own menu lists the chords (⌘W closes a pane).
- Measurements and deviations from the Penpot design: [docs/app](../../docs/app/README.md).

## Tests

`cargo test -p orch-app`: the core on a real socket (a `PermissionRequest` held until the Inbox
answers, then `allow` on the wire), a second core refused on the same home, shells pulled through
a PTY, Governor controls (pause all, undo), the planner curve against the Governor's snapshot, the
Inbox ladder, kitty import, editor templates, menu chords, settings.

UI: `cd ui && npm test` (vitest + jsdom): keyboard contract, tiling layout, Inbox triage, status
vocabulary, fuzzy match, formatting, the token pipeline (drift and semantic-only check), and
components (sidebar, title bar, status bar, Inbox keyboard triage with undo).
