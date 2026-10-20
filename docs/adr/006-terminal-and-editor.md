# ADR-006 — Terminal & editor: kitty-looking panes, user's own nvim

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (PTY in Rust, keyboard contract)

## Context
The user lives in kitty + nvim. Agents themselves are Claude Code TUIs in terminal panes (ADR-002), so terminal quality is central. The goal is that panes **render and feel like kitty**: same font, colors, ligatures, cursor and spacing. Reproducing kitty's protocols is not a goal.

## Decision
- **Rendering**: xterm.js in the webview (Unicode 11; ligatures only when kitty enables them). The **DOM renderer is the default**, because the first WebGL pane costs about 110 MB in WebKit's GPU helper. WebGL is opt-in, capped at 12 visible panes (WebKit allows 16 contexts per page). Hidden panes drop their WebGL context and keep only their buffer (SPEC FR-1005).
- **PTYs**: `portable-pty` in the Rust core, running the user's **login shell**, so PATH, dotfiles, nvim plugins, LSPs and fzf behave as usual. Output reaches the UI through the pull transport of ADR-001 (Channels stalled under load in P0).
- **kitty look**: settings imported from `kitty.conf` (following `include`s): font family/size, `modify_font`/line height, colors, cursor, padding, opacity. Unmapped options are listed. The P0 terminal spike showed that matching kitty takes four fixes:
  1. Resolve `font_family` the way kitty does: exact monospace family, otherwise Menlo. On this machine the configured `FiraCode-Regular Nerd Font` doesn't resolve, so kitty actually renders Menlo; the app warns about it.
  2. The core finds font files with CoreText and hands them to the UI (`FontFace`), because WKWebView hides user-installed fonts.
  3. Use kitty's bundled `SymbolsNerdFontMono` as the symbol fallback.
  4. Round the cell width up and take the line height from CoreText metrics, as kitty does.

  Result: near pixel-identical side by side; kitty's text is slightly heavier, which only shows when zoomed in.
- **Keyboard contract** (SPEC §9.7): when a pane has focus, every key goes to the PTY except ⌘-chords and the leader key, **`⌘;`** by default (`Ctrl-Space` clashes with macOS input-source switching and nvim-cmp). Shift+Enter sends `ESC CR` in agent panes. The app ships its own menu so system items don't take its chords. Known gaps: no kitty keyboard protocol, so `Ctrl-I` = Tab; `Ctrl-/ . ;` and `Ctrl-Shift-<letter>` send nothing.
- **Editor**: no in-app code editor. "Open in editor" runs `nvim +{line} {file}` (configurable; defaults to kitty.conf's `editor` if set) in a pane, reusing the project's nvim through `--server/--remote` when one is open. External kitty is optional, through `kitty @ launch --cwd` when remote control is enabled. nvim with the full user config was verified in P0: truecolor, SGR mouse, `"+` clipboard.

## Consequences
- The user keeps their exact toolchain and look.
- The `kitty.conf` mapping is best effort; the differences are shown in settings.
