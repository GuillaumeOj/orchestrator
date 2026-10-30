# Improve Agent Pane · API contract

> Companion of [spec.md](spec.md). **No new command and no payload change.** Moving agents into the dock is UI only (the layout is the UI's own `settings_set('layout', …)` value).

## Commands used (unchanged signatures)

| Command | Args | Returns | Notes |
|---|---|---|---|
| `agent_attach` | `id, rows, cols` | reader `u64` | Marks the agent attached and makes its idle TUI redraw at `rows × cols`. |
| `agent_detach` | `id, reader` | — | Only the current reader detaches. |
| `agent_read` | `id, reader` | bytes | Pull transport. |
| `agent_resize` | `id, rows, cols` | — | A plain PTY resize. |
| `term_spawn` / `term_resize` / `term_kill` | as today | | Shells. |

## Agent redraw on show (T-63: UI side, no core change)

Cause confirmed (code reading): `agent_attach`'s `cols-1` / `cols` resizes land back to back, and Node's tty emits `'resize'` only when the size it reads differs from the last one it saw, so a pane at the PTY's previous size (reopen, restore, resume) gets no redraw; a re-shown tab (remount, same size) sends nothing at all.

- `agent_attach` is unchanged (same signature, its `cols-1` / `cols` step kept) but the UI does not rely on it for the redraw.
- The UI owns the forced redraw on **every show** of an agent (first open, tab switch back, dock shown, resume, restore): `NUDGE_MS` after the show, `agent_resize(rows-1, cols)`, then after a gap (`REDRAW_GAP_MS`) `agent_resize(rows, cols)`. Once per show; none for a pane hidden before it fires; the PTY is never left at `rows-1` (a hide during the gap sends the real size at once).

## Layout value (UI only, `settings` key `layout`)

`dock.tabs` may now hold agent panes (`{ type: 'agent', agentId, projectId }`). Restore moves agent panes found in `layout` (the tiling tree) into `dock.tabs`. Older app versions reading a newer value: not supported (local single-user app).
