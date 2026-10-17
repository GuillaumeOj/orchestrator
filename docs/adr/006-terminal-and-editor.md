# ADR-006 — Terminal & editor: kitty-looking embedded terminal, user's own editor

- Status: Proposed · Date: 2026-10-17

## Context
The user lives in kitty + nvim with a full personal configuration and does not want a built-in editor. The goal for the terminal is that it **renders and feels like kitty**: same font, colors, ligatures, cursor and spacing. The goal is not to reproduce kitty's protocols or every feature.

## Decision
- **Terminal**: xterm.js in the webview (WebGL renderer, ligatures, Unicode 11) on a PTY owned by the Rust core (`portable-pty`, ADR-001). It runs the user's **login shell**, so PATH, dotfiles, nvim plugins, LSPs, fzf and the rest behave as usual.
- **kitty look**: read the user's `kitty.conf` (font family/size, `modify_font`/line height, color scheme, cursor shape and blink, window padding, background opacity) and apply it to xterm.js. A manual theme editor covers anything that can't be mapped.
- **Editor**: no in-app code editor. "Open in editor" runs a configurable command template (default `nvim +{line} {file}`) in an **embedded terminal pane**. As an option, it can open in external kitty or reuse a running nvim through `--server/--remote`.
- **Open in kitty**: a convenience action that opens the same cwd in the real kitty. It is not needed for normal use.

## Consequences
- The user keeps their exact toolchain, and the app's terminal looks like their usual one.
- The `kitty.conf` mapping is best effort. Unmapped options are listed in the settings so the user knows what differs.
