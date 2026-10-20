# Terminal spike (P0 item 5)

Throwaway Tauri 2 app: a Rust core owns the PTYs (`portable-pty`, the user's login
shell) and streams output to xterm.js 6 (WebGL, ligatures, Unicode 11 addons) in
WKWebView. UI in Svelte 5. Findings: [docs/spikes/p0-terminal.md](../../docs/spikes/p0-terminal.md).

```sh
npm install
npx tauri build            # release .app in src-tauri/target/release/bundle/macos/
SPIKE_CTL=/tmp/orch-term-spike.sock SPIKE_LOG=/tmp/spike.log \
  src-tauri/target/release/bundle/macos/OrchTerminalSpike.app/Contents/MacOS/orch-terminal-spike
./ctl.sh '{"cmd":"new","n":6}'       # drive it over the control socket
```

Environment knobs:

| Variable | Effect |
|---|---|
| `SPIKE_CTL` | Unix socket for JSON-line commands (see `src/ctl.js`: `new`, `tab`, `type`, `run`, `frames`, `state`, `keytest`, `webgl_stress`, `dropHidden`, `webgl`, `eval`, …). |
| `SPIKE_LOG` | File where the UI's `report` lines and core events are appended. |
| `SPIKE_KEYLOG` | Log every byte string written to a PTY. |
| `SPIKE_KITTY_CONF` | kitty.conf to import (default `~/.config/kitty/kitty.conf`, read-only). |
| `SPIKE_FONT` | Force a font family (used for the FiraCode comparison). |
| `SPIKE_LIGATURES` | Load the ligatures addon even if kitty disables ligatures. |
| `SPIKE_COALESCE_MS` | Channel mode: coalesce PTY reads for up to N ms per message. |

Transport modes: Tauri `Channel` push with byte-count acks (default), or pull
(`window.spikePull = true` before creating panes): the UI calls `pty_read`, which
returns everything buffered as one raw response.

`scripts/` holds the harness used for the report (benchmarks, memory, screenshots).
They expect `S` to point to a scratch directory containing `tools/winid` (built from
`winid.swift` with `swiftc`) and `demo/` (with `glyphs.sh`, `demo.rs` and a large
`big.txt`). Rough on purpose: this is a throwaway spike.
