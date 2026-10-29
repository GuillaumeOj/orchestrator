# Fix Shell Pane On Opening · Feature spec

> Feature `fix-shell-pane-on-opening` · Integration branch `orch/fix-shell-pane-on-opening/integration` · UI first (no API change expected, no design pass)

## 1. Problem

When the user opens a shell, the terminal is not drawn as it should be: parts of what runs in it are missing (the command line is not at the bottom, a TUI's side pane, such as its diff view, is not shown, and so on). As soon as the user resizes the window or zooms the pane, the terminal is redrawn correctly.

**Goal:** a shell is drawn at its real size the moment it opens, with no resize or zoom from the user.

## 2. How a shell opens today (HEAD 68ea76d)

1. `openShell` / `runInPane` / `openEditor` / `revive` (`ui/src/lib/app.svelte.js`) spawn the PTY in the core with a **guessed size** (`term_spawn` with 24×80, or 30×100), then add the pane (workspace split or dock tab). The program in the PTY starts drawing at that guessed size right away.
2. `TermPaneView` mounts, then `mountTerm` → `TermPane.mount` (`ui/src/lib/term.js`): `term.open`, `applyLineHeight` (it may change `lineHeight`, so the cell height), then **one** `fitNow`, then a `ResizeObserver` on the host that calls `scheduleFit` (60 ms debounce).
3. `TermPane.start()` sends the xterm's current size to the PTY (`term_resize`) and starts pulling output. Later fits reach the PTY through `term.onResize`.

`fitNow` returns early, silently, when the host has no size yet, when `proposeDimensions()` has no cell metrics yet, when the size is unchanged, or when the A→B→A flip guard triggers.

Resizing the window or zooming always works because both produce a **real size change of the host**: the `ResizeObserver` fires (or the pane remounts), xterm fits, the PTY gets SIGWINCH and the program redraws.

## 3. Likely causes (to confirm before fixing)

The fix owner first confirms which of these happens (instrumented build, logs of `fitNow` decisions and of the `term_resize` calls), and fixes the real one(s), not all of them blindly:

- **H1 · Fit before the final layout, no later notification.** The single fit at mount runs while the pane's box is not final (pane just inserted in a split, dock just opened), or is skipped (no size, no cell metrics yet), and the next layout change does not trigger the observer, or is dropped by a guard.
- **H2 · Cell metrics change after the fit.** `applyLineHeight` or a font finishing loading changes the cell size *after* the fit, without the host's box changing: xterm's rows/cols no longer match the box (rows hidden at the bottom), and no observer fires.
- **H3 · The PTY never gets the final size, or gets it too late.** The program draws at 24×80 (or 30×100); a resize is not sent when xterm's size did not change between mount and `start()`, or arrives while the program is still starting, and nothing triggers a redraw afterwards.

## 4. Scope

- After a shell opens, without any user action, the xterm grid matches its box (cols and rows), and the PTY is told that size, so the program redraws for it. This holds for every way a terminal pane opens:
  - a dock tab (`openShell()`, ⌘J with no shell, the dock's `+`);
  - a split (`openShell({ how: 'right' | 'below' })`, the project pane's **Shell** button, the agent's **Shell here**);
  - `runInPane` (gh auth login, claude /mcp, trust);
  - an editor pane (nvim) from `openEditor`;
  - a shell restored with the layout at app start (`revive`).
- The fix lives where terminals are sized: `TermPane` (`term.js`) and `mountTerm` (`app.svelte.js`). Agent panes share that code and must keep working (and benefit from the same fix).
- Keep the 60 ms debounce for window drags and the flip guard against TUIs that bounce between two sizes; adjust them only if one of them is the cause, and say so.
- No change to the core's PTY code unless the investigation shows it is needed (then `back` owns it and it goes into `api-contract.md` first).

Out of scope: the terminal look (font, theme, padding), the dock and split layout themselves, agent attach behavior.

## 5. Acceptance

1. Opening a shell in the dock, in a split, from a project and from an agent shows the prompt at the right place and a full-size grid immediately; a TUI started at once (e.g. `run` in `runInPane`, nvim from `openEditor`) fills the pane with its bottom line visible, without resizing the window or zooming.
2. Resizing the window, dragging the dock, zooming and unzooming still refit as today (no flicker loop, no regression for agent panes).
3. A shell restored at app start is drawn at its real size.
4. A unit test in `ui/test/termhost.test.js` (or a new `ui/test/term.test.js`) covers the confirmed cause: e.g. the pane refits and sends `term_resize` with the final size when the box or the cell metrics settle after mount. `bun run test`, `bun run check`, `bun run lint` pass.
5. The task report names the confirmed cause (H1, H2, H3 or another) and how it was confirmed.

## 6. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Shell drawn at its real size on open | front | `plan_first`: root cause + fix plan before coding. `term.js`, `mountTerm`, test |

`back` reviews the frontend change, and owns a core change only if the plan shows one is needed.
