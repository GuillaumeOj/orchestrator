# Update Orchestrator UI · API contract

> Companion of [spec.md](spec.md). The front-end state keys are those of `.orchestrator/features/improve-orchestrator-ui-ux/api-contract.md` and design-notes §5.4 (unchanged, not repeated). This file fixes what `back` adds and what `front` calls.

## 1. Terminals (`crates/orch-app`)

| Item | Shape | Notes |
|---|---|---|
| `Term::cwd(&self) -> Option<PathBuf>` (`terms.rs`) | cwd of the **shell's pid** (not the foreground group) | macOS `proc_pidinfo(PROC_PIDVNODEPATHINFO)` → `pvi_cdir.vip_path`; elsewhere `readlink /proc/<pid>/cwd`; `None` if the process is gone; check the child has not exited first (`try_wait`) so a reused pid is never read |
| Tauri command `term_cwd(id: u32) -> Option<String>` (`commands.rs`, next to `term_resize`; registered after `term_kill`) | `null` when the terminal id is unknown or exited | never an error for a closed terminal: the UI polls it |
| Shell name | `term_spawn` result `TermInfo` (`core.rs`) gains `shell: String` = basename of the program actually spawned for an interactive shell (e.g. `"zsh"`; same source as the spawn, so it can't disagree with `login_shell()` / the default program) | for command terminals (`run` set) it is still the shell basename; the UI ignores it there |

UI: polls `term_cwd` every 1500 ms per live dock shell while `document.visibilityState === 'visible'`, skips a shell once OSC 7 was seen (design-notes §6).

## 2. Projects

| Command | Args | Result | Rules |
|---|---|---|---|
| `rename_project` | `{ id, name }` | the updated project record | name trimmed, 1–80 chars, unique among the other projects (case-insensitive, the project being renamed excluded, so keeping or re-casing its own name is allowed; two projects may already share a folder-derived name) → error text the UI shows under the field: "A project named <name> already exists" / "Name can't be empty". Emits the existing project-changed topic so the sidebar, tabs and windows refresh. Write only the `name` / `default_branch` column (a dedicated store update), not `upsert_project` with a copy read earlier: that upsert overwrites every column and would undo a concurrent edit |
| `set_default_branch` | `{ id, branch }` | the updated project record | `branch` must be a local branch of the project's repo (`git rev-parse --verify refs/heads/<branch>`) → else error "No branch <branch> in this repo". Writes `projects.default_branch`. **Settled:** the General field writes `projects.default_branch`. `ProjectSettings.base_branch` stays an optional per-project override used only to pick a feature's base (`team.rs`: spec base, else `base_branch`, else `default_branch`); it is not the project's default branch and is not touched here |
| `project_branches` (new; no equivalent existed) | `{ id }` | `string[]` local branches, default first | feeds the Default branch Select |

If equivalent commands already exist, back reuses them and lists their names in the report; front calls what the report names.

## 3. Settings

| Key | Type | Default | Read / write |
|---|---|---|---|
| `memory_warn_mb` (UI setting in the `ui.` key-value store; the live-cap and memory-pressure settings are governor policy, a different store) | `u32`, MB, 256–65536 | `1600` | read with `settings_get`, written with `settings_set` (an unvalidated key-value write): **the UI validates** the range and shows "Between 256 and 65 536 MB"; back adds nothing for it. Not the same as the governor's `memory_warn` pressure state nor the per-WebView `ORCH_WEBKIT_WARN_MB` (watch.rs, 600 MB): the Memory section labels which total it compares (`AppMemory.total_mb`). The Memory section shows the warning state when the current sample total is above it. No other behaviour depends on it in this feature |
| `notifications` | exists | — | UI only (Overwatch › Notifications switches) |

## 4. Native menu (`menu.rs:ITEMS`)

- `pane.focus.1…9`: labels "Tab 1…9" (ids unchanged).
- `pane.close`: "Close Tab". `pane.reopen` (if present): "Reopen Closed Tab".
- Remove menu items for `pane.split.right`, `pane.split.below`, `pane.zoom`, `focus.left`, `focus.right` if present. `focus.up` / `focus.down` (leader `k` / `j`) are **kept**: they move between the workspace and the dock (design-notes §4.1).
- New **Help** submenu with `("shortcuts", "Keyboard Shortcuts", "CmdOrCtrl+/")`, registered as the app's Help menu on macOS. Front adds `'cmd+/': 'shortcuts'` to `CHORDS` and normalises `/` in `chordName` (AZERTY, design-notes §3.4): the webview handles the chord first, so the native accelerator not firing on a layout where `/` needs Shift is accepted.
- Extend the `menu.rs` test (or add one) for these labels and ids.
