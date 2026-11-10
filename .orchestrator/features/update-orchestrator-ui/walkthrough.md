# Update Orchestrator UI · Acceptance walkthrough (T-317)

> Walked by `ux` on the integration branch at `c74fa6a4` (after T-316), against [acceptance.md](acceptance.md). Method: each check read in the code (`ui/src`) and matched with the test that covers it; frames spot-checked (`04-settings-terminal-editor-no-kittyconf-1280`); `grep` for `<select`, `<Kbd`, `addPane`. The app was **not launched**: runtime and pixel-level checks (spacing, colour, hover states, a real `cd` in a shell) stay with `ui`'s per-task reviews and a manual pass. No code changed.
> Gates run here: `bun run test` 1043 / 1044 (the one failure is `chat.test.js` "renders a now event in < 16 ms", a timing assertion, 35 ms in this sandbox, unrelated to this feature); `bun run lint` exit 0 (165 biome warnings, none fail); `bun run check` 0 errors, 0 warnings. `grep -rn "<select" ui/src` is empty. Rust tests were not run.
> Verdicts: **pass** · **deviation** (D-n, listed in the summary) · **clarify** (acceptance and design disagree; lead decides).

## Summary of deviations, most serious first

| # | Where | Rule / frame | Expected | Found |
|---|---|---|---|---|
| D-1 | F1.3 · `Dialogs.svelte:addProject` → `app.svelte.js:openProject` | dn §5.1; `03-empty-workspace-project` | After Add, the context becomes the new project; with no tab the empty state shows **Open <project>** + **New session** | `addProject` only calls `openProject(p.id)`: a Project tab opens and the context is not changed (stays All on a first launch). The empty state is never reached from F1. Fix: `openProjectContext(p.id)` (sets the context, then opens the pane), or `setContext` without opening the pane |
| D-2 | F6.5 · `app.svelte.js:decide` (`always`), `PermissionCall`, chat card, agent strip | `06-flow-decision` step 5; dn §3.3 | "Always allow" asks a confirm dialog (window shell, `Cancel · esc`) | No dialog: every Always allow goes through the 5 s undo toast (`pendingToast`). Lead decides: add a `confirm(...)` for `always`, or accept the undo toast and amend flows.md F6 step 5 |
| D-3 | F11.4 / dn §9 bug 5 · `PlannerWindow.svelte:176` vs `app.svelte.js` palette `governor.pause-all` vs `Banner.svelte:59` | dn §9 bug 5: one handler, one toast | One handler and one toast from every Pause all | Planner button and key share one handler (✓), but the palette command toasts a different text ("Paused all · resumes on Resume all" vs "Paused all · Resume all from the palette") and the banner's **Pause all** toasts nothing. Fix: one `pauseAll()` in `app.svelte.js` used by the three |
| D-4 | Surfaces › Banner · `Banner.svelte` (`claudeDetails`, `update`) | audit §3; acceptance "only `claude` Set up, ×; no `Details`" | `claude`: Set up, × | A too-old / failed Claude Code adds **Update** and **Details** buttons (`bannerClaude`, from the Claude Code update feature). Probably intended; lead decides whether acceptance or banner changes |
| D-5 | Surfaces › Status bar · `StatusBar.svelte` | audit §8: connection, pressure, ☾, counts; PR build and update chips | Information only | No connection item (only the core error banner). Extra: git branch / ahead-behind / dirty, a clickable Claude Code chip (`openSettings('claude')`), `RAM` with a WebKit warning. These pre-date the redesign and look intended; list them in the audit or drop them |
| D-6 | Surfaces › Settings footers · `AllocationSettings.svelte:255`, `McpSettings.svelte:433`, `IntakeSettings.svelte:543` | "Revert label on every footer" (`SettingsFooter`) | Save + **Revert**, both disabled until dirty | Allocation footer reads **Cancel**; the MCP server form has Save only (Test / Delete beside it); Intake shows Revert only while dirty (not disabled). `SettingsFooter` (General, Task backend, Notion) is right |
| D-7 | Surfaces › Team canvas · `TeamCanvasPane.svelte` (`.menu` popovers: Connect, Add a node, Starter, Duplicate, Library) | spec goal 6: shared `Menu` replaces every ad-hoc popover menu | `Menu` | Hand-written popovers with `Kbd` in `role="menuitem"` rows. `hints.test.js` allows the `Kbd` there, so no key pill rule is broken; only the shared-Menu goal is not met. Low |
| D-8 | F9.6 · `FloatingWindow.svelte:onWindowKey` | acceptance: "Esc → closes" | — | With a section nav, Esc inside the content returns to the nav first; a second Esc closes. This is the documented behaviour of the component (dn §3.3) and the acceptance line is the shorter one: **clarify** and amend acceptance |
| D-9 | F13.4 · `TerminalSettings.svelte` | acceptance lists "Terminal settings" among the Open in kitty places | — | The section has **Open kitty.conf** (opens the file in an nvim tab), which is what the frame draws. No kitty launcher there: **clarify** and amend acceptance |

No other deviation found. The rows listed under "Left out (spec §3.1)" are absent as intended.

## Part 1 · Flows

### F1 `start` · First launch to first agent
1. Setup pane, app-wide, no project highlighted: **pass** (`restoreLayout` opens Mission control + Setup when not done; `paneProject(setup)` null).
2. Add project dialog in the window shell, Cancel / Add tooltips, no `Kbd`: **pass** (`Dialogs.svelte`, `Sidebar` `+` tooltip `Add project · ⌘N`).
3. Context becomes the project; empty state: **deviation D-1**. The empty state itself is right (`Workspace.svelte`: "⌘K to open anything", **Open <project>**, **New session**).
4. New session dialog with Selects: **pass** (`noselect.test.js`, `Dialogs.svelte`).
5. Start → dock agent tab, view Agents, filter kept or moved to its project: **pass** (`openAgent` → `addDockPane` → `focusPane`).
6. Agent header: identity, StatusBadge, model, ctx, Interrupt + Suspend (live), `⋯` last, no Resume, no hint: **pass** (`AgentPane.svelte:139-159`).

### F2 `context` · Work in one project
1. All projects: every tab, nothing highlighted, header "every project · follows the project context": **pass** (`inContext`, `Sidebar` / `IconRail` `current`, `MissionPane:323`).
2. Project click → context + Mission control stays: **pass** (`openProjectContext`, app-wide panes in every context).
3. Open feature → new tab, full title, context unchanged: **pass** (`addPane` + `focusPane` keeps a context that shows it).
4. Select → other project: first tab unless active is app-wide; empty state otherwise: **pass** (`setContext`, `settleActive`; §12.8 no per-context last tab).
5. All + open a pane from the sidebar → All stays: **pass** (`focusPane` only changes the context when the tab is hidden).
6. Select options, counts, ✓, waiting dot, tooltip names the agent: **pass** (`contextOptions`, `contextWaiting`, `waitingText`; `context.test.js`).
7. Mission control empty in a project: "Nothing in <project>" + **Show all projects**, no Filter button / legend: **pass** (`MissionPane:337-340`).
8. Palette "Set the project context…", `intake.refresh` / "Plan with the Overwatch…" follow the context: **pass** (`commands()`: `mission.filter` hidden while the dock fills the column, as designed).

### F3 `tabs` · Many tabs
1. Tabs never shrink, `‹ ›`, `‹` disabled at start: **pass** (`TabStrip.svelte`, `tabstrip.test.js`).
2. `›` scrolls by width − one tab, fades only on a clipped edge: **pass** (`scrollStep`, `fade-l` / `fade-r`).
3. Off-screen tab scrolled into view on open: **pass** (`idKey` effect).
4. ⌘n = n-th visible tab, beyond the count nothing, no toast: **pass** (`focusNumber`).
5. ⌘W closes the window first, then the tab; ⌘⇧T reopens: **pass** (`run('pane.close')`, `reopenClosed`; windows are not in the closed stack).
6. Tab icon, full title, × hover/active, middle-click, roving keys: **pass** (`onLaneKey`, `onAux`).
7. No pane header, split handle, zoom, number: **pass** (`Workspace.svelte` renders `PaneHost` only).

### F4 `dock` · Agents and shells in the dock
1. View Select "Agents n", project Select, tabs, `‹ ›`, Hide dock `· ⌘J`, no `+` in Agents: **pass** (`Dock.svelte`, `dockViewOptions`).
2. Filter keeps the view counts: **pass**.
3. Waiting dot on the project Select only, tooltip "<agent> (<project>) is waiting for you": **pass** (`dockProjectWaiting`).
4. In Shells the dot moves to the view Select, never two Selects for one agent: **pass** (`dockViewWaiting`).
5. Shell title follows the cwd: poll 1.5 s, OSC 7 first, left-truncated at 32, tooltip full path, command shells keep their title: **pass** by code and `shellcwd.test.js` (`trackShells`, `readShellCwd`, `liveTitle`, `shellTip`). Real `cd` not run.
6. Last shell closed: "No shell open" + inline **New shell**, no `+`: **pass** (`Dock.svelte`). Note: closing the last shell while agent tabs exist switches to Agents first (dn §5.2, F4.9); "No shell open" shows when the user picks Shells with none. Consistent with dn §5.2.
7. "No agent running in <project>": **pass** (`emptyText`).
8. Opening an agent / shell switches the view, view restores its tab, leader `]` `[` cycle within view + filter: **pass** (`focusPane`, `settleDock`, `cycleDock`).
9. ⌘J with no tab opens a shell; closing the last tab of a view switches or closes: **pass** (`toggleDock`, `closePane`).

### F5 `agent-state` · A suspended agent
1. Banner has no agent condition: **pass** (`Banner.svelte` kinds are app-wide).
2. Sidebar row StatusBadge, no caret on agent rows; dock tab same vocabulary: **pass**.
3. One **Resume**, in the strip: **pass** (`AgentPane` `resume` snippet; `AgentMenu` and `AgentInspector` have none).
4. `⋯` Menu items and order, disabled with reason, Kill / Delete destructive with confirm: **pass** (`AgentMenu.svelte`).
5. Waiting: strip text + WaitingChip (+ Resume when not live); header chip only without a strip: **pass** (`{#if !strip}<WaitingChip>`).
6. Strips per state, live / not live, blocked tooltip "Blocked until the reset · hh:mm", "also in Decisions": **pass** (`AgentPane:163-229`; `components.test.js`).
7. Inspector: details + Open session + `AgentMenu`: **pass**.
8. Watch bar Take over `· ⏎`, Back `· esc`: **pass**.

### F6 `decision` · Answer a decision
1. Title bar Decisions icon / label / count, one gap token, tooltip `· ⌘⇧I`: **pass** (`TitleBar.svelte`).
2. Chat column handle: **pass** (`chatwidth.test.js`; not drawn over).
3. Decision card Allow; strip says "also in Decisions" (focuses the card): **pass**.
4. After Allow the status changes in dock and sidebar: **pass** (shared `statusOf`).
5. "Always allow" confirm dialog: **deviation D-2**.
6. Chat placeholder `Message the Overwatch · ⏎ to send`, no `Kbd`: **pass** (`ChatInput.svelte:65`).

### F7 `feature` · A feature from plan to PR
1. Header: Pause/Resume, Stop, Delete (danger), tooltips + `aria-label`, `⋯` last: **pass** (`FeaturePane:281-285`; `headers.test.js`).
2. Doc path line only for a doc in the repo, Open in nvim icon: **pass** (`docpath.test.js`, `.docpath`).
3. PrLine: PR link, CI dot + `n/m`, review state: **pass** (`PrLine.svelte`; `prline.test.js`).
4. CI popover lists checks with state, footer link to GitHub checks: **pass** (`ciFooter`, `/checks`). Opens on hover and on click.
5. `⋯` = Board, Canvas, Publish / Refresh in Notion, Open in Notion: **pass** (`FeaturePane:201-218`).
6. No Board button in the Tasks card: **pass**.
7. Message box: To Select, input, Send icon: **pass** (`:376`; same height by R11 `Select` regular).
8. RunInspector PR through PrLine, no `Open PR ↗`: **pass** (`RunInspector:62`; a text-only fallback link when the run has no `pr_url`).

### F8 `project` · Project pane actions
1. Gear, bin (danger), `⋯` last: **pass** (`ProjectPane:151-153`).
2. `⋯` = Fetch, Shell, Diff <default branch>, Trust in a pane (untrusted only): **pass** (`:124-127`; label shows the branch name, e.g. "Diff main").
3. Worktree row `±` and bin icons with tooltips: **pass** (`:195-196`).
4. Remove worktree confirm: **pass** (`removeWorktree`).
5. Gear opens Project settings, one window at a time: **pass** (`openProjectSettings`, `setWindow`).

### F9 `settings` · App settings window
1. ⌘, / gear, 960 (944 at 1024), `role="dialog"`, ⌘, again closes: **pass** (`windowSize`, `toggleWindow`; `floatwindow.test.js`).
2. 9 sections, no `Kbd`, no Keyboard section, every one built: **pass** (`SettingsWindow.SECTIONS`).
3. Integrations links update the window in place: **pass** (`openSettings` on an open window).
4. Deep links with server / PR preselected, no new tab: **pass** (`openSettings(section, server)`; Banner, StatusBar PR chip).
5. Edit a key: input + Save + Cancel, Remove / Replace on the row: **pass** (`IntegrationsSettings`; `settings-sections.test.js`).
6. Esc closes, an open Select consumes it first, focus returns to the opener: **pass** with **clarify D-8** (Esc in content goes to the nav first).
7. Section variants: Claude Code, MCP, Terminal & editor (no-kittyconf matches the frame), Pacing, Memory (default 1600), Overwatch Notifications, Permissions, Builds without a channel Select: **pass** by code and `settings-sections.test.js`, `terminalsettings.test.js`, `memorysettings.test.js`; pixel match not checked.
8. Autosave sections close freely; footer sections ask "Discard changes?": **pass** (`guardWindow`, `unlessDirty`). Footer label issue: **D-6**.
9. Focus trap: **pass** (`inert` + `onTrapKey`).

### F10 `project-settings` · Project settings window
1. 7 sections, General first, opens on Task backend: **pass** (`ProjectSettingsWindow.SECTIONS`, `openProjectSettings` default `backend`).
2. General: Name and Default branch only, Save + Revert, untrusted shows Trust in a pane: **pass** (`ProjectGeneral.svelte`, `projectgeneral.test.js`).
3. Linear not connected → link replaces the window, Discard guard: **pass** (`openSettings('integrations')` through `setWindow`).
4. Files access Select, missing-file state: **pass** (`noselect.test.js`, `FilesSettings`).
5. Each section saves itself: **pass** (`saveBackend` / `saveNotion` send the saved other half; bug 1).
6. MCP overrides: **pass with a known change**: Inherit / Override only, Off hidden (spec §3.1 amended 2026-11-10); deep link with project opens it (`openSettings(..., projectId)`).
7. Allocation "Edit weekly plan…" opens the plan dialog above the window, Cancel only: **pass**; the section footer says Cancel not Revert, see **D-6**.
8. Intake, Notion as drawn, Select = input height: **pass** (Intake Revert only when dirty, see D-6).
9. Close, one window at a time: **pass**.

### F11 `planner` · Week planner
1. PacingIndicator popover with **Week planner**: **pass** (`PacingIndicator:72`).
2. Window 1100 wide (app − 80 at 1024): **pass** (`PlannerWindow` size, `windowSize`); also from leader `g` and banners.
3. Plan features, Defer / Keep running, Pause all, report, Shadow / Sprint, Governor log with Undo: **pass** (existing planner content moved into the window).
4. Pause all same toast from button and key: **pass** inside the planner; **deviation D-3** across planner, palette, banner.
5. No planner entry in the status bar: **pass** (`StatusBar`, `PressurePill` information only).
6. Weekly report button, palette `report.list` opens the list: **pass** (`openReport('list')`).

### F12 `shortcuts` · Find a key
1. Tooltips `Label · key`, no pill: **pass** (`tooltip.js`; `hints.test.js`).
2. ⌘/ (AZERTY handled), Help menu, palette; two columns, `Leader = ⌘;`: **pass** (`chordName`, `ShortcutsSheet`, `lib/shortcuts.js`).
3. Search filters rows, empty groups hide, "cmd" matches ⌘: **pass** (`shortcuts.test.js`).
4. Palette rows with bindings: **pass** (`Palette.svelte:53`).
5. Empty pane line "⌘K to open anything": **pass**.
6. `LEADER` only while ⌘; held: **pass** (`StatusBar:61`).
7. No `.kbd` beside or inside a control outside the allowed places: **pass** (`grep`: `Menu`, `Palette`, `ShortcutsSheet`, `StatusBar` leader, `TeamCanvasPane` menu rows; `hints.test.js` enforces).

### F13 `terminal` · Shell and editor
1. New shell (leader `t`, dock `+`) in the focused worktree, title `zsh · <cwd>`: **pass** (`openShell`, `liveTitle`).
2. File from agent output → nvim workspace tab, project-bound, no split: **pass** (`openEditor` → `addPane` editor with `projectId`).
3. Terminal & editor: theme Select, kitty switch, read-only Terminal, missing kitty.conf text, hidden rows absent: **pass** (`TerminalSettings.svelte`).
4. Open in kitty from agent `⋯`, project `⋯`/Shell: **pass** (`AgentMenu` kitty; project `⋯` Shell). Terminal settings: **clarify D-9**.
5. Leader `v s h l z` dead, `j k` kept, no `pane.split.*` / `pane.zoom` command: **pass** (`LEADER` map, `commands()`).

### F14 `migrate` · Restart and old layouts
1. Saved keys `workspace.project`, `dock.view/agentProject/activeAgent/activeShell`, `sidebarTree`: **pass** (`saveLayout`).
2. Relaunch restores context, dock view + filter, active tabs, tree; unknown project → All; vanished keys pruned: **pass** (`restoreLayout`, `knownContext`, `pruneTree`, `prunePanes` → `setContext('all')`). Note: `pruneTree` runs at restore only; a feature deleted while the app is closed is pruned because `refresh()` precedes `restoreLayout` in `App.svelte`.
3. `activeAgent` outside the filter → first agent of the filter or empty; `dock.view` absent → kind of active: **pass** (`settleDock`, `view` fallback).
4. Old split layout → one tab strip, saved focus active; settings / project-settings / planner panes dropped: **pass** (`L.flatten`, `WINDOW_KINDS`; `layout.test.js`, `persist.test.js`).
5. Closed windows not offered by ⌘⇧T: **pass** (`closePaneByUser` is for panes only).
6. Not restored on purpose (windows, scroll, popovers): **pass**.

## Part 2 · Surfaces (audit §8)

- **Title bar**: [x] Jump search field-button `· ⌘K` · [x] PacingIndicator · [x] Decisions count · [x] no pills.
- **Banner**: [x] app-wide kinds only, no `Later`, no `Kbd` · [ ] `claude` shows Update / Details beyond Set up: **D-4**.
- **Sidebar and icon rail**: [x] `+ Add project · ⌘N` · [x] project / feature / agent rows · [x] carets on project and feature only, remembered (`toggleTree`) · [x] no `⌘B` pill · [x] nothing highlighted in All.
- **Workspace tab strip**: [x] context Select (compact, hugs, max 220, counts, dot) · [x] tabs · [x] `‹ ›` only on overflow · [x] no pane header.
- **Dock strip**: [x] all items; `+` only in Shells with at least one shell (tooltip "New shell", no key, trivial).
- **Status bar**: [ ] information only, but no connection item and extra Claude chip / git / RAM: **D-5** · [x] PR build and update chips only when present · [x] `LEADER` only while held · [x] pressure and ☾ are information.
- **Agent pane**: [x] header and strips as in F5.
- **Agent inspector**: [x].
- **Run inspector**: [x] blocker link, members, PrLine, Open feature, Open board.
- **Project pane**: [x].
- **Feature pane**: [x].
- **Board**: [x] feature chips, Sync, New task · [x] card menu with right-aligned keys (`BoardPane:menuItems`; "Open remote" present when the task has a remote) · [x] no footer legend (the footer shows the selected task only).
- **Mission control**: [x] no Filter button or legend · [x] follows the context · [x] Ready lane follows it · [x] Ready row buttons with tooltips.
- **Team canvas, Diff, Report, Setup, dialogs**: [x] no legends; Connect / Delete / Duplicate (`⇧D`) as inspector buttons with tooltips · [x] dialogs in the FloatingWindow shell, Cancel / submit tooltips · [x] native selects gone · [ ] popovers are not the shared Menu: **D-7**. Design preview `×`: not walked.
- **Settings, Project settings, Week planner, Shortcuts**: [x] all four floating windows, no `addPane` for them (`grep addPane`: Mission, report, project, feature, board, canvas, diff, editor only) · [x] 9 / 7 sections · [x] hidden rows absent · [x] Select height, compact only in the two strips · [x] `noselect.test.js` · [x] one model list (`lib/options.js`) · [ ] Revert on every footer: **D-6** · [x] Discard guard on dirty footers.
- **Whole app**: [x] `grep "<select"` empty · [x] `<Kbd` only in allowed places (`hints.test.js`) · [~] `make lint` UI part passes, `make test` UI part 1043 / 1044 (timing flake), Rust not run · [x] bugs 1–4, 6, 7 fixed; bug 5 partly: **D-3**.
