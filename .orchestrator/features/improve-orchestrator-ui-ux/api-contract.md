# Improve Orchestrator UI UX · State contract

> Companion of [spec.md](spec.md). This file fixes the front-end state the follow-up implementation adds, and its one backend addition (`term_cwd`, below), so the design and the notes use the same names. Details: [design-notes.md](design-notes.md) §5–§6.

## Persisted UI state (saved with the layout, `saveLayout` / `restoreLayout`)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `sidebarTree` | `{ [projectId \| "f:<featureId>"]: boolean }` (expanded; the keys `Sidebar.svelte` already uses) | expanded | tree state of the sidebar (replaces the in-memory `collapsed` of `Sidebar.svelte`; not `sidebar`, which is the existing show/hide boolean, app.svelte.js:984) |
| `workspace.project` | `projectId \| "all"` | the focused project, else `"all"` | project context of the workspace tabs |
| `dock.view` | `"agents" \| "shells"` | `"agents"` if an agent tab exists, else `"shells"` | dock view |
| `dock.agentProject` | `projectId \| "all"` | `"all"` | agents filter |
| `dock.activeAgent`, `dock.activeShell` | pane id \| null | the first tab of the view | active tab per view; `dock.active` becomes the active tab of the current view, so switching views brings back each view's last tab |

A missing or unknown project id falls back to `"all"`.

## Layout migration

A saved layout with splits (`app.layout` tree) restores as one tab strip, panes in reading order (left→right, top→bottom), the focused one active. (`app.zoomed` is never saved: nothing to migrate; the code and its commands go.)

## Shell title

- The shell tab title is `<shell> · <cwd>` from the terminal's **OSC 7** (`file://host/path`) sequence; when a shell sends none, the **OSC 0/2** title (xterm `onTitleChange`); else the opening path (today's behaviour).
- zsh and bash don't send OSC 7 by default. **Decided** (design-notes §6.1): a core-side poll of the **shell process's** cwd, OSC 7 honoured when a shell sends it; no shell integration injected.
- New Tauri command `term_cwd(id) -> Option<String>` (`crates/orch-app/src/commands.rs`, next to `term_resize`): the cwd of the shell's pid (`proc_pidinfo PROC_PIDVNODEPATHINFO` on macOS, `/proc/<pid>/cwd` elsewhere), `None` when the terminal is gone. The UI polls it every 1.5 s per live dock shell while the window is visible.
