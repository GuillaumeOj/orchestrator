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

## Possible change (only if T-2's plan confirms cause A1)

`agent_attach` guarantees the redraw: the TUI sees a real size change (the `cols-1` step and the final size are not both applied before the program reads the size), e.g. the final resize delayed after the first one. Same signature and return. The UI may then rely on `agent_attach` for the first show, and on a UI-side nudge (`agent_resize` to `rows-1`, then `rows`, with a gap) for a re-show; whichever side owns the nudge, the plan says so and this file is updated before coding.

## Layout value (UI only, `settings` key `layout`)

`dock.tabs` may now hold agent panes (`{ type: 'agent', agentId, projectId }`). Restore moves agent panes found in `layout` (the tiling tree) into `dock.tabs`. Older app versions reading a newer value: not supported (local single-user app).
