# Settings sections brief (R14)

> Round 2, T-295. For ui's T-296: one frame per section of both windows, in the "Improve UI UX" row of page 04.7, at 1280 and one at 1024 (Settings › Integrations and Project settings › Files already exist: redo them to the rules below). Companion of [spec.md](spec.md) §7, [ux-brief.md](ux-brief.md), [audit.md](audit.md).
> Sources: `ui/src/panes/SettingsPane.svelte`, `ProjectSettingsPane.svelte`, `ui/src/components/*Settings.svelte`, `docs/SPEC.md` (FR-203, FR-904, FR-1002, FR-1005, FR-1102), `ui/src/lib/app.svelte.js` (`theme`, `kittyTheme`, `notifications`).

## 0. Rules for every section

1. **Window**: the existing floating window (design-notes §2.5). Left nav of sections, content right, scrolls inside. Nav item: label, optional count/status on the right (kept: Claude Code update dot/arrow, Integrations and MCP "needs you" counts). **No `Kbd` in the nav** (the `j`/`k` hint and the `⌘,` pill go; the key is in the tooltip of the gear / the palette).
2. **Section = title + one-line lead (muted) + groups.** A group is a card (`surface-pane`, subtle border, radius md) with an `h3`-style header and rows. One row = label (fixed 168 px column) · control · muted help text under or beside it. No more than one primary button per section.
3. **Save model, by section type**:
   - *Autosaved* (each change applies at once, a small "Saved" confirmation next to the control for 2 s): everything that is a choice among options or a toggle (theme, notifications, cap, renderer…).
   - *Form with a footer* (primary **Save**, secondary **Revert**, both disabled until dirty, "Unsaved changes" in muted text): sections that edit text the user types (Task backend, Intake, Notion, Allocation, General name/path). The footer sticks to the bottom of the content. No `Kbd` on the buttons (key in tooltip: `Save · ⌘⏎`).
4. **Selects** are the shared Select (design-notes §2.1); a Select next to a text input has the input's height (R11). Segmented controls only for 2–3 short options that are a mode (Backend: Local / Linear / GitHub).
5. **Status** is a badge (dot + word), never colour alone. Danger actions are a ghost button in `danger` colour at the end of their own group ("Danger zone"), with the confirm dialog as today.
6. **Deep links** keep opening on the section (`openSettings(section, server)`; design-notes §3.2). A "needs you" row inside a section gets the amber `status-waiting-for-you` dot and is listed first.
7. Every section has four states drawn at least once across the set: loading (skeleton rows), empty, error (inline `err` box with Retry), and normal. Draw them where they apply (listed per section).
8. Each row says **where the value lives**: "stored" = already stored today (no backend change); "new" = needs a store/command the frontend team must add; the frame carries a small `new` tag only in the design notes, not on screen.

## 1. Settings (app-wide), 9 sections

The `Keyboard` section is gone (it is the Keyboard shortcuts window, ⌘/). Order in the nav: Claude Code, Integrations, Overwatch, Permissions, MCP servers, Terminal & editor, Pacing policy, Memory, Builds. No "General" section is added: appearance goes into Terminal & editor, notifications into Overwatch.

### 1.1 Claude Code (today: version block, update block, compatibility, account; keep, restructure)

| Group | Rows | Source |
|---|---|---|
| **Installed** | version (mono) · channel/meta · path (copy on click, tooltip "Copy path") · status badge (Up to date / Update available / Update failed / Updating…) | stored (`ClaudeSettings`, `ClaudeUpdateBlock`) |
| **Update** | the update block as today: **Check now** (ghost), **Update to x.y.z** (primary, only when available), progress and last result; "Agents run with auto-update off. The app updates Claude Code only when you ask." as help text | stored |
| **Compatibility** | Supported range · notes list (▲ amber / ● red) | stored |
| **Account** | Login · note (e.g. "Max subscription") · **Sign in again** secondary only when login is missing | stored |
| **Defaults for new sessions** *(new)* | Default model (Select: the model list scoped as design-notes §2.2), Default effort (Select), Permission mode (Select) | **new**: the role defaults live in the Node config today (FR-303); this group edits the app-wide fallback. If the frontend team judges it out of scope, drop the group; nothing else depends on it |

States: normal (up to date), update available, update failed (amber badge + error text + Retry), not installed (`head.missing`: empty state with the install hint and **Check again**).

### 1.2 Integrations (today: list of GitHub, Linear, Notion, … with status and a key field)

Keep the row-per-integration list; redraw as cards, one per integration: name, status badge (Connected / Needs you / Not connected / Error), account/scope line, **Connect / Replace key / Disconnect** (ghost; Disconnect danger) and the key field (password input, "Show" icon button, **Save key** primary only when the input has text). Needs-you cards first. Full texts, no truncation (the T-291 nit). States: loading (3 skeleton cards), empty can't happen, error (a card with `err` and Retry), one card expanded (key being edited), one saving.

### 1.3 Overwatch (today: OverwatchSettings: policy, answer scope, …) + **Notifications** group

Groups: **Policy** (what the Overwatch answers alone: the existing options, as Select/segments), **Answering from the docs**, **Notifications** (moved here from the palette-only `notify.desktop` / `notify.sound` toggles: two switches "Desktop notifications", "Sound", stored under `notifications`). Autosaved. States: normal, Overwatch off (rows disabled with the reason as tooltip).

### 1.4 Permissions (today: PermissionsSettings: policy files, defaults when no rule matches, hold)

Groups, as the code has them: **Policy files** (one row per file: the global one and one per project; path, title, parse error or warnings, **Open in editor** / **Create and open**; rules are edited in the file, not in a table), **When no rule matches** (two Selects, Team roles and Solo sessions: Allow / Ask me / Deny as `EFFECTS` lists them; a muted line when the Overwatch is off), **Hold** (hold seconds: number input with `s` suffix, whole number, default 300, `0` = do not hold, the prompt stays in the agent's pane; must stay above the Overwatch's permission time; key `permissions.hold_secs`, autosave on blur). States: no policy file yet, a file with a parse error (last good version still used), warnings list with "Show n more", hold rejected (error under the input, previous value kept). A rules table is **not** in this section: it would be a new editor for the policy files, out of scope.

### 1.5 MCP servers (today: McpSettings: registry list, per server detail, "needs you" count)

Master/detail inside the section: list on the left (name, status dot, scope badge), detail on the right (command/URL, env as key/value rows with show toggle, per-project overrides link to Project settings › MCP overrides, **Test connection**, **Remove** in the danger zone). Add: **Add server** in the list header. States: loading, empty ("No MCP server registered"), a server with an error, a server with a missing secret (needs you).

### 1.6 Terminal & editor (placeholder today; **real design**)

Everything here already exists in the app, but is changed only by the palette or by editing `kitty.conf`. Groups:

| Group | Rows | Source |
|---|---|---|
| **Appearance** | Theme (Select: Follow the system / Dark / Light), **Derive colours from kitty.conf** (switch; a user choice of Dark/Light wins, as today) | stored (`app.theme`, `app.kittyTheme`; palette ids `theme.*`) |
| **Terminal** | Font (read-only mono line: resolved family and size, source "kitty.conf", with **Open kitty.conf** in editor as ghost), Cursor, Padding, Opacity (read-only line each, from the import), Unmapped options (muted list "n options not applied", expandable) | stored (`app.kitty`, FR-1002) |
| **Renderer** | Renderer (Select: DOM (default) / WebGL), help: "WebGL costs about 110 MB for the first pane. At most 12 visible panes use it." | **new** setting (FR-1005 says "WebGL is opt-in (setting)"); if it does not exist yet, show the row disabled with "Coming later" tooltip rather than hiding it |
| **Editor** | Editor command (text input, default `nvim +{line} {file}`, mono, **Reset** ghost), "Reuse the project's open nvim" (switch, default on), **Open in kitty** terminal cwd help | stored/new: FR-1102 defines the template; the input is **new** unless `editor` is read from `kitty.conf` (then show it as read-only with "from kitty.conf") |
| **Shells** | Default shell (read-only: `$SHELL`), new shell opens in (Select: focused agent's worktree / project root) | stored (behaviour of FR-1003); the Select is **new** |

Autosaved; text inputs apply on blur/Enter. States: normal; no `kitty.conf` found (group shows "No kitty.conf found. Defaults are used." and the font rows say "Menlo (default)").

### 1.7 Pacing policy (today: PacingSettings, 349 lines: live cap, weekly/5 h thresholds, off-hours, AC power)

Groups in this order: **Live agents** (cap stepper 1–20, default 3, key `governor.live_cap`; help "Applied at once; lowering never stops a working agent"), **Weekly and 5-hour limits** (thresholds as sliders with the number to the right; the guard chip behaviour as help), **Off-hours** (switch + window From/To time inputs + "only on AC power" switch), **Catch-up**. Autosaved with the "Saved" confirmation. States: normal, a value outside range (error), Governor in Red (read-only banner at the top of the section "Governor is in Red until 13:00": information, no button).

### 1.8 Memory (placeholder today; **real design from FR-904 / NFR-1b**)

Information-first section; the app already measures its memory (status bar) and the pressure level. Groups:

| Group | Content | Source |
|---|---|---|
| **Now** | Total (Tauri + webview + each `claude`), as a stacked bar: App (Rust core + webview + GPU helper) / agents (one segment each, labelled by id) / shells; pressure state badge (Normal / Warn / Critical) with the since-time; the live agent count vs cap | stored (status bar's memory and pressure, FR-904) |
| **Warning** | Warn above (number input, MB; default 1 600 MB), **Show in the status bar** (switch: the memory chip) | the threshold is **new** (FR-904 says "with a warning threshold"); show disabled if absent |
| **Under pressure** | read-only explanation list: "warn: one live agent less (min. 1) · critical: 1 · back up after 120 s of normal"; link "Live agents cap" → Pacing policy | stored behaviour (FR-203); text only |
| **Renderer** | link to Terminal & editor › Renderer (no duplicate control) | — |

States: normal, warn (amber bar segment + badge), critical (red), measuring unavailable (muted "Not available on this system").

### 1.9 Builds (today: BuildsSettings: PR builds, update channel; selected server deep link)

Keep the structure; apply the rules of §0 (cards, no Kbd, Select for channel). One group **PR builds** (list with status dots, **Install** per build), one **Updates** group (channel Select, **Check now**, last checked). States: loading, none ("No PR build"), error.

## 2. Project settings (per project), 7 sections

Window title "Project settings · <project>". Nav order: **General, Files, Task backend, Intake, Allocation, Notion, MCP overrides**. The project name sits in the window title and in a small identity line at the top of General only.

### 2.1 General (placeholder today; **real design from the project record and its pane**)

Today these live on the project pane (card) and in the sidebar menu; the pane header keeps identity and the icon actions (R4), General is where they are *edited*.

| Group | Rows | Source |
|---|---|---|
| **Project** | Name (input; rename), Path (mono, read-only, **Reveal in Finder** / **Open in kitty** ghost icon buttons), Id (muted, copy) | stored (`Project.name`, `path`, `id`) |
| **Git** | Default branch (Select of the repo's branches, current marked), Remote (read-only, `origin` url), Fetch every 15 min (read-only help: "paused on low battery or offline"), **Fetch now** (secondary), Last fetch time + error | stored (`default_branch`, `last_fetch_*`); the fetch interval is fixed (help text, not a setting) |
| **Trust** | Status badge (Trusted / Not trusted yet) and **Trust in a pane** (secondary, only when untrusted) | stored (`d.trusted`, `openTrust`) |
| **Worktrees** | Worktree root (read-only mono), "Remove merged worktrees automatically" (switch) | root stored; the switch is **new** — drop the row if the team does not want it |
| **Danger zone** | **Remove project…** (ghost danger) with its confirm; text "Sessions and history stay on disk" as it says today | stored (`startRemoval`) |

Form with footer (Name, Default branch are the only dirty-able fields). States: normal, untrusted, fetch error (amber row), rename conflict (error under the input).

### 2.2 Files (exists; redo to the rules)

Per-file access (the CLAUDE.md-like files and the docs folder the agents may read/write): list of rows, file path (mono) · access Select (Read & write / Read only, `ACCESS` in `lib/projectfiles.js`; a click toggles it in code today) · note · **Open in nvim** icon · remove icon. **Add file…** at the list's end (dropping from Finder also adds). **Autosaved per action, no footer** (`FilesSettings` has no Save today): it is the exception to the form-with-footer rule of §0.3. States: normal, empty, a path that no longer exists (amber row "Missing" + Remove).

### 2.3 Task backend (exists)

Segmented Local / Linear / GitHub; per kind a group: Linear (connection badge + link to Settings › Integrations, Team Select, Project Select, state → column mapping as a 2-column table of Selects, "Move tasks to Linear?" as its own group with **Move** primary-ghost and a count). Footer Save/Revert. States: not connected (info box + link), loading teams (skeleton), error, mapped.

### 2.4 Intake (exists, #48: `IntakeSettings`, global defaults in `IntakeGlobalSettings`)

Keep its content; apply the rules (cards, footer, no Kbd). Group the existing controls as **Sources**, **Rules**, **Schedule**; "Inherit from the app default" shown as a muted chip with **Override** (ghost) per row; an overridden row shows **Reset to default**. States: inheriting everything, one override, error on a source.

### 2.5 Allocation (exists: `AllocationSettings`)

Groups, as the code has them: **This project's share** (Weight input, Tier Select: critical / high / normal / low, Hard cap % input, the cap bar against the weekly use), **Rest of the plan** (read-only table of the other projects' shares, or "Proposed split" before the plan is confirmed), **Keep in the plan / Remove** (a weight of 0 until Save). Footer Save/Revert. States: normal, plan not confirmed (proposed split, read only), removal pending, a value out of range (error under the input). There are no per-role overrides and no concurrency stepper here.

### 2.6 Notion (exists, small)

One group: Parent page (input, URL or id), help "Share the page with your integration", link to Settings › Integrations for the token, status line (token present: badge). Footer.

### 2.7 MCP overrides (exists: `McpSettings` with a project)

Same master/detail as 1.5 but each row shows **Inherit / On / Off** (three-option segmented) and, when On, the server's per-project env. No add/remove here (that is Settings › MCP servers; link at the top).

## 3. Frames to draw (T-296)

Naming `04-settings-<section>-1280.png`, `04-project-settings-<section>-1280.png`; plus `-1024` for Claude Code, Terminal & editor, Memory, Project General, Task backend. One frame per section is the normal state; add the listed *additional states* as separate frames only for: Claude Code (update available, failed), Integrations (editing a key), Terminal & editor (no kitty.conf), Memory (warn), Pacing (Red), Project General (untrusted), Files (missing file), Task backend (not connected), MCP servers (error). Everything else: one frame.

## 4. What the brief does not decide

- Whether the **new** rows (Defaults for new sessions, WebGL renderer, editor template, default shell location, memory warn threshold, worktree auto-remove) ship. Each is marked **new** above and drawn in the frame; the implementing feature picks which to build and hides the others. None changes another screen.
- Settings search (a field in the nav): not drawn; sections are few.
