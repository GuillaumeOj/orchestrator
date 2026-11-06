# Allow Overwatch Menu Resizing · Feature spec

> Feature `allow-overwatch-menu-resizing` · Integration branch `orch/allow-overwatch-menu-resizing/integration` · UI only (no core, no API change) · small design pass (ui)

## 1. Problem

The Overwatch (the chat column on the right of the shell, `ChatColumn`) has a fixed width: 360 px at ≥ 1512 px, 340 px below (`ui/src/App.svelte` `.chatslot`, token `--size-chat`). Long Overwatch lines, cards and source links wrap in a narrow column, and on a big screen the user cannot give it more room; on a small one they cannot give the centre panes more room.

**Goal:** the user drags the left edge of the Overwatch column to make it wider or narrower; the width is remembered across restarts.

## 2. Current code (HEAD 5882f8b)

- `ui/src/App.svelte`: `.mid` is a flex row: Sidebar/IconRail · `.work` (flex: 1, workspace + dock) · `.chatslot` (flex: none, `width: var(--size-chat)`, `.narrow` → 340 px) · docked Inspector (≥ 1728 px, 320 px). Below 1728 px the Inspector overlays `.chatslot`, so it takes the slot's width.
- `ui/src/lib/app.svelte.js`: `app.width` (window width), `wide()` (≥ 1512), `saveLayout()` / `restoreLayout()` persist the shell layout under the settings key `layout` (generic JSON, `settings_set`).
- `ui/src/components/Dock.svelte`: the existing resize pattern to copy: a 4 px `role="separator"` handle, pointer drag on `window`, arrow keys nudge, `saveLayout()` on release.

## 3. Scope

1. **A resize handle** on the left edge of the Overwatch slot (`.chatslot`), full height, same look and behaviour as the dock's handle but vertical (`cursor: col-resize`, `aria-orientation="vertical"`). Details in `ux-brief.md`.
2. **Drag** sets the column width in px (pointer moves left = wider). Live while dragging; no text selection, no iframe/xterm stealing the pointer during the drag (pointer capture or a full-window drag shield).
3. **Keyboard**: the handle is focusable; `←` / `→` widen / narrow by 16 px (`⇧` 64 px); `Home` / `End` go to min / max; `⏎` or double-click resets to the default.
4. **Bounds**: min 300 px; max = the smaller of 720 px and what leaves the work column at least `--size-centre-min` (640 px) beside the sidebar and a docked Inspector. The bound is applied at render (window resize, sidebar toggle, Inspector docking) **without overwriting** the saved width, so the column grows back when the window does. When even the min does not fit, the min wins (as today, the shell may get tight).
5. **Default**: no saved width = today's behaviour exactly (360 / 340 by breakpoint). A reset clears the saved width (back to the breakpoint default). A saved width applies at every breakpoint.
6. **Persistence**: `app.chatWidth` (number | null), saved by `saveLayout()` in the `layout` value as `chatWidth`, restored by `restoreLayout()` (invalid / missing → null). Save on drag end and after a keyboard change (the existing 400 ms debounce), not on every pointer move.
7. **Inspector overlay** (< 1728 px) keeps covering the slot at its current width; the handle stays usable while the overlay is open.
8. The App.svelte header comment ("fixed, 360 px; 340 below 1512") and `ChatColumn.svelte`'s ("a fixed column") are updated.

Out of scope: collapsing / hiding the Overwatch, resizing the sidebar or the docked Inspector, per-project widths, a menu command.

## 4. Acceptance

1. Dragging the Overwatch's left edge resizes it live; the centre panes (workspace, dock, terminals) reflow; xterm panes refit on release (or live) without garbage.
2. The width cannot go below 300 px, above 720 px, nor squeeze the work column under 640 px; making the window smaller shrinks the column, making it larger again restores the chosen width.
3. The width survives an app restart; with no saved width the column is 360 / 340 as today.
4. Keyboard: Tab reaches the handle (visible focus), arrows / Home / End / ⏎ work; double-click resets.
5. Hover and drag states match the design (`design/` exports).
6. Tests: a pure clamp helper (e.g. `chatWidthBounds` / `clampChatWidth` in `ui/src/lib/app.svelte.js` or a small `lib/chatwidth.js`) unit-tested; save/restore of `chatWidth` tested next to the dock's layout tests (`ui/test/dock.test.js` style); a component test that the separator renders with its aria attributes. `bun run test`, `bun run check`, `bun run lint` pass.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| T-1 Resize handle design | ui | — | Draw the handle's rest / hover / dragging / focus states and the column at min, default and a wide width on the Overwatch board in the Drive `Orchestrator.figcraft`; export PNGs to `design/` |
| T-2 Resizable Overwatch column | front | T-1 | `App.svelte`, `lib/app.svelte.js` (state, clamp, save/restore), handle (inline or `components/chat/ChatResize.svelte`), tests |

No backend work (the `layout` setting is free-form JSON). No UX task: `ux-brief.md` (written by lead) is the behaviour reference; `ui` turns it into the visuals.
