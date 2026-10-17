# ADR-006 — Terminal & editor: kitty-looking panes, user's own nvim

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (PTY in Rust, keyboard contract)

## Context
The user lives in kitty + nvim. Agents themselves are Claude Code TUIs in terminal panes (ADR-002), so terminal quality is central. The goal is that panes **render and feel like kitty**: same font, colors, ligatures, cursor and spacing. Reproducing kitty's protocols is not a goal.

## Decision
- **Rendering**: xterm.js in the webview (WebGL renderer, ligatures, Unicode 11). Hidden panes drop their WebGL context and keep only their buffer, to stay under WebKit's context limit and save memory.
- **PTYs**: `portable-pty` in the Rust core, running the user's **login shell**, so PATH, dotfiles, nvim plugins, LSPs and fzf behave as usual. Output streams over binary channels.
- **kitty look**: settings imported from `kitty.conf`: font family/size, `modify_font`/line height, colors, cursor, padding, opacity. Unmapped options are listed.
- **Keyboard contract** (SPEC §9.7): when a pane has focus, every key goes to the PTY except ⌘-chords and one configurable leader key. P0 checks the known gaps against kitty (xterm.js doesn't implement the kitty keyboard protocol, e.g. Ctrl-I vs Tab).
- **Editor**: no in-app code editor. "Open in editor" runs `nvim +{line} {file}` (configurable) in a pane, reusing the project's nvim through `--server/--remote` when one is open. External kitty is optional.

## Consequences
- The user keeps their exact toolchain and look.
- The `kitty.conf` mapping is best effort; the differences are shown in settings.
