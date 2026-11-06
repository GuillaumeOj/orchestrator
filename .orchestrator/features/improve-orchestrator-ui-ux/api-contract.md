# Improve Orchestrator UI UX · State contract

> Companion of [spec.md](spec.md). No backend or IPC change is designed in this feature. This file fixes the front-end state the follow-up implementation adds, so the design and the notes use the same names.

## Persisted UI state (saved with the layout, `saveLayout` / `restoreLayout`)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `sidebar.expanded` | `{ [projectId \| "projectId/featureSlug"]: boolean }` | expanded | tree state of the sidebar (replaces the in-memory `collapsed` of `Sidebar.svelte`) |
| `workspace.project` | `projectId \| "all"` | the focused project, else `"all"` | project context of the workspace tabs |
| `dock.view` | `"agents" \| "shells"` | `"agents"` if an agent tab exists, else `"shells"` | dock view |
| `dock.agentProject` | `projectId \| "all"` | `"all"` | agents filter |

A missing or unknown project id falls back to `"all"`.

## Layout migration

A saved layout with splits (`app.layout` tree) restores as one tab strip, panes in reading order (left→right, top→bottom), the focused one active. `app.zoomed` is dropped.

## Shell title

- The shell tab title is `<shell> · <cwd>` from the terminal's **OSC 7** (`file://host/path`) sequence; when a shell sends none, the **OSC 0/2** title (xterm `onTitleChange`); else the opening path (today's behaviour).
- zsh and bash don't send OSC 7 by default: the implementation decides between the shell integration snippet the app injects at spawn and a core-side cwd poll of the PTY's foreground process. Not designed here; the notes state the choice.
