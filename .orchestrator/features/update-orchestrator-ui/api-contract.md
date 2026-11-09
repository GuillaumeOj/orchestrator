# Update Orchestrator UI · API contract

> Companion of [spec.md](spec.md). The front-end state keys are those of `.orchestrator/features/improve-orchestrator-ui-ux/api-contract.md` and design-notes §5.4 (unchanged, not repeated). This file fixes what `back` adds and what `front` calls.

## 1. Terminals (`crates/orch-app`)

| Item | Shape | Notes |
|---|---|---|
| `Term::cwd(&self) -> Option<PathBuf>` (`terms.rs`) | cwd of the **shell's pid** (not the foreground group) | macOS `proc_pidinfo(PROC_PIDVNODEPATHINFO)` → `pvi_cdir.vip_path`; elsewhere `readlink /proc/<pid>/cwd`; `None` if the process is gone |
| Tauri command `term_cwd(id: u32) -> Option<String>` (`commands.rs`, next to `term_resize`; registered after `term_kill`) | `null` when the terminal id is unknown or exited | never an error for a closed terminal: the UI polls it |
| Shell name | the term spawn result (or `TermInfo`, whichever the UI reads at open) gains `shell: String` = basename of `login_shell()` (e.g. `"zsh"`) | for command terminals (`run` set) it is still the shell basename; the UI ignores it there |

UI: polls `term_cwd` every 1500 ms per live dock shell while `document.visibilityState === 'visible'`, skips a shell once OSC 7 was seen (design-notes §6).

## 2. Projects

| Command | Args | Result | Rules |
|---|---|---|---|
| `rename_project` | `{ id, name }` | the updated project record | name trimmed, 1–80 chars, unique among projects (case-insensitive) → error text the UI shows under the field: "A project named <name> already exists" / "Name can't be empty". Emits the existing project-changed topic so the sidebar, tabs and windows refresh |
| `set_default_branch` | `{ id, branch }` | the updated project record | `branch` must be a local branch of the project's repo (`git rev-parse --verify refs/heads/<branch>`) → else error "No branch <branch> in this repo". Writes `projects.default_branch`. Back checks whether the typed setting `base_branch` (`intake.rs:base_branch`) already plays this role and, if so, documents in its report which one the General field writes and why (one field, one source) |
| `project_branches` (if no equivalent exists) | `{ id }` | `string[]` local branches, default first | feeds the Default branch Select |

If equivalent commands already exist, back reuses them and lists their names in the report; front calls what the report names.

## 3. Settings

| Key | Type | Default | Read / write |
|---|---|---|---|
| `memory_warn_mb` (app setting, next to the live-cap/memory settings) | `u32`, MB, 256–65536 | `1600` | read with the existing settings getter, written with the existing setter; out-of-range → error "Between 256 and 65 536 MB". The Memory section shows the warning state when the current sample total is above it. No other behaviour depends on it in this feature |
| `notifications` | exists | — | UI only (Overwatch › Notifications switches) |

## 4. Native menu (`menu.rs:ITEMS`)

- `pane.focus.1…9`: labels "Tab 1…9" (ids unchanged).
- `pane.close`: "Close Tab". `pane.reopen` (if present): "Reopen Closed Tab".
- Remove menu items for `pane.split.right`, `pane.split.below`, `pane.zoom`, `focus.left`, `focus.right` if present.
- New **Help** submenu with `("shortcuts", "Keyboard Shortcuts", "CmdOrCtrl+/")`. Front adds `'cmd+/': 'shortcuts'` to `CHORDS`.
- Extend the `menu.rs` test (or add one) for these labels and ids.
