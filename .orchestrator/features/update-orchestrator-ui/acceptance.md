# Update Orchestrator UI · Acceptance checklist

> Used by `ui` to review each front task and by `ux` for the T9 walkthrough. Companions: [spec.md](spec.md), [ux-brief.md](ux-brief.md).
> Sources (on main): `improve-orchestrator-ui-ux/` → `flows.md`, `audit.md` §8, `design-notes.md` (`dn`), `settings-brief.md`. PNGs are in its `design/` folder; the storyboard of flow `<id>` is `06-flow-<id>.png`, the other frames are named by their step.
> Format: `n. action → expected` · *PNG* · rule. "Test:" = the vitest file that covers the flow (dn §10, §13.4); a flow also needs the manual walk.
> Left out (spec §3.1, hidden rows): Claude Code › Defaults for new sessions; Terminal & editor › Renderer, Editor command, Reuse the open nvim, New shell opens in; Memory › Show in the status bar; Project › General › Remove merged worktrees automatically. Their absence is not a deviation, and no empty card or gap remains.
> Sample data (project `orch`, `fusily`, feature `feat-x`, agents `lead`, `back-1`, `ui-1`) is illustrative.

## Part 1 · Flows

### F1 `start` · First launch to first agent
Storyboard `06-flow-start.png`. Test: none (manual).
1. Fresh launch, no project → Setup pane (app-wide), no project highlighted · *04 Setup (no frame; rules of dn §8 "Panes with no frame")* · app-wide pane.
2. Add project (Sidebar `+`, tooltip `Add project · ⌘N`, or ⌘N) → dialog in the FloatingWindow shell; Cancel/Add carry no `Kbd`, tooltips `Cancel · esc` · dn §3.3.
3. Confirm → context becomes the project; no tab → empty state: line "⌘K to open anything" + **Open <project>** + **New session** · *03-empty-workspace-project* · dn §5.1.
4. New session (⌘T or button) → dialog with Select for project / role / model / effort / permission mode (no native select) · dn §2.1.1.
5. Start → agent tab appears in the dock, view **Agents**, filter unchanged (or to its project only if hidden) · *03-dock-agents-filtered* · dn §5.2 Opening.
6. Agent pane opens in the dock; header = identity, StatusBadge, model, ctx, Interrupt + Suspend (live), `⋯` last; no `Resume`, no `⏎`/`esc` hint · *04-agent-strips* · dn §1.3.1.

### F2 `context` · Work in one project
Storyboard `06-flow-context.png`. Test: `context.test.js`, `sidebar.test.js`, `mission-pane.test.js`.
1. Context All projects → every tab shows, Mission control active, header "every project · follows the project context"; no project row or icon-rail mark highlighted · *03-context-all-projects* · dn §5.1 rule 4.
2. Click project `orch` in the sidebar → context `orch`, tabs filtered, Mission control (app-wide) stays visible; Select shows `orch` · *03-app-1280*, *03-context-select-open* · dn §5.1 rules 1, 4.
3. Open feature `feat-x` → new tab with its full title; context unchanged · *02-tab-strip* · dn §2.3.
4. Select → `fusily` → its first tab activates, unless the active tab is app-wide (Mission control stays active); with no tab: empty state with **Open fusily** + **New session** · *03-context-select-open*, *03-empty-workspace-project* · dn §12.8.
5. Context All, open a `fusily` pane from the sidebar → context stays All · dn §5.1 rule 3.
6. Context Select: options All projects + projects, counts = visible tabs, ✓ on current, waiting dot per dn §12.7, tooltip names the agent · *03-context-select-open*.
7. Mission control in a project with nothing → "Nothing in <project>" + **Show all projects** (sets All); no Filter button, no toolbar legend · *04-mission-control-context* · dn §5.3.
8. Palette "Set the project context…" opens the Select; `intake.refresh` / "Plan with the Overwatch…" follow the context, no throw · dn §5.1 rule 7.

### F3 `tabs` · Many tabs
Storyboard `06-flow-tabs.png`. Test: `tabstrip.test.js`, `layout.test.js`, `keyboard.test.js`.
1. 12 tabs at 1280 → tabs never shrink, `‹ ›` appear, `‹` disabled at start · *03-tab-overflow-1280* · R6.
2. `›` → scrolls by width − one tab; both enabled, fade on the clipped tab only, nothing drawn between it and the buttons, buttons opaque · *03-tab-overflow-states* · R6.
3. Open a tab that is off-screen → strip scrolls it into view; at the end `›` disabled · *03-tab-overflow-states*.
4. ⌘3 → third **visible** tab of the context; ⌘n beyond the count does nothing, no toast · dn §4.1.
5. ⌘W closes the focused tab (with a window open it closes the window); ⌘⇧T reopens the closed tab · dn §4.1, §3.3.
6. Tab: icon, full title (ellipsis > 280), × on hover/active only, middle-click closes, ← → Home End roving · *02-tab-strip*.
7. No pane header, no split handle, no zoom, no pane number anywhere · dn §4.3.

### F4 `dock` · Agents and shells in the dock
Storyboard `06-flow-dock.png`. Test: `dockviews.test.js`, `dock.test.js`, `shellcwd.test.js`.
1. Dock Agents, All projects → view Select "Agents n" (n = open dock tabs of the kind, all projects), project Select, tabs (role mark, short name, StatusBadge, ×), `‹ ›`, Hide dock icon (tooltip `Hide dock · ⌘J`), no `+` · *03-app-1280*, *02-dock-strip* · dn §5.2.
2. Filter to `orch` → only its agents; counts unchanged on the view Select · *03-dock-agents-filtered*.
3. An agent of another project needs you → dot on the **project** Select only, tooltip "<agent> (<project>) is waiting for you"; none on the view Select · *03-dock-agents-filtered* · dn §12.7.
4. Switch to Shells → two shells with live titles `zsh · ~/dev/orch/ui/src/components`; dot now on the **view** Select (not on the project Select); one waiting agent never lights two closed Selects · *03-dock-shells* · dn §12.7.
5. `cd` in a shell → title follows within ~1.5 s (OSC 7 instant); long path left-truncated at ~32 chars; tooltip = full path; command shells keep their fixed title · *03-dock-shells*, *02-dock-strip* · dn §6.
6. Close the last shell → "No shell open" + inline **New shell**, no `+` in the strip · dn §5.2 Empty view.
7. Agents filtered to a project with none → "No agent running in <project>" · dn §5.2.
8. Opening an agent switches the view to Agents; opening a shell to Shells; switching view restores that view's active tab; leader `]` / `[` cycle within view + filter · dn §5.2.
9. ⌘J with no tab at all opens a shell; closing the last tab of a view switches to the other view or closes the dock · dn §5.2.

### F5 `agent-state` · A suspended agent, one home per action
Storyboard `06-flow-agent-state.png`. Test: `components.test.js`, `surfaces.test.js`, `hints.test.js`.
1. Suspended agent → banner shows only app-wide conditions (none for the agent) · *04-agent-suspended* · dn §1.3.1.
2. Sidebar → agent row shows its StatusBadge (agent rows are leaves, no caret); dock tab shows the same vocabulary · dn §12.6, §5.2.
3. Agent pane strip → **Resume** is the only Resume on screen (not in header, not in Inspector) · *04-agent-strips*, *04-agent-suspended* · dn §8.
4. Header `⋯` → trigger drawn pressed while open, Menu right-aligned under it; items: Shell in its worktree, Diff against base, Open in kitty | Run off-hours (checkable) | Restart fresh…, Kill…, Delete… (destructive, confirm dialog); disabled items give a reason · *04-agent-suspended*, *04-agent-suspended-menu-closed* · R7.
5. Agent waiting for you → strip carries text + WaitingChip (+ Resume when not live); header has no WaitingChip while a strip shows; Inspector has details only · *04-agent-waiting*, *04-agent-inspector* · dn §8.
6. Strips per state (live / not live): permission (Allow once, Deny, Always allow, Answer in the pane or Resume when not live, "also in Decisions" link), queued (Run now + cost, Cancel), blocked (Resume disabled, tooltip "Blocked until the reset · hh:mm"), error, suspended · *04-agent-strips* · dn §1.3.1.
7. Inspector: details + Open session + same `⋯` Menu (`AgentMenu`); no Resume, Interrupt, Suspend · *04-agent-inspector* · dn §12.5.
8. Watch bar: Take over (tooltip `Take over · ⏎`) + Back to Mission control (`Back · esc`); ⏎ / esc still work · *04-agent-strips*.

### F6 `decision` · Answer a decision
Storyboard `06-flow-decision.png`. Test: `hints.test.js`, `surfaces.test.js`.
1. Title bar Decisions → icon, label, count with one gap token; tooltip `Decisions · ⌘⇧I`; no pill · *03-title-status-banner* · R5.
2. Open Decisions → Overwatch chat column; its resize handle works and nothing is drawn over it · *03-app-1280* · R1.
3. Decision card → **Allow** (once); the card and the agent strip are the two homes, strip says "also in Decisions" (link focuses the card) · *04-agent-strips* · dn §8.
4. After Allow → agent resumes, status dot/StatusBadge changes in the dock and sidebar · StatusBadge vocabulary.
5. Policy "Always allow" → confirm dialog (window shell, tooltips `Cancel · esc`, no pill) · dn §3.3.
6. Chat input placeholder `Message the Overwatch · ⏎ to send` only; no `Kbd` · dn §7.

### F7 `feature` · A feature from plan to PR
Storyboard `06-flow-feature.png`. Test: `headers.test.js`, `prline.test.js`, `docpath.test.js`.
1. Feature pane running → header: identity left, icon buttons Pause/Resume, Stop, Delete (danger colour), each with tooltip + `aria-label`, then `⋯` last · *04-feature-pane*, *04-feature-pr-running* · R9.
2. Doc tabs → muted repo-relative path line (only for a doc inside the repo) with Open in nvim icon button at its end; no text button · *04-feature-pane* · R10.
3. Review done → PrLine in the team column: PR link, CI dot + `n/m`, review-done state · *04-feature-pr-review-done* · R13.
4. PR open → click CI dot: checks popover lists checks with state + "Open checks on GitHub" · *04-feature-pr-running* · R13.
5. `⋯` → Board, Canvas, Publish/Refresh in Notion, Open in Notion; Publish to Notion works · *04-feature-pane* · R9.
6. Board from `⋯` → board tab; the Tasks card has no Board button · *04-feature-pane* · dn §8.
7. Message box: To Select + input at the same height + Send icon button · R11.
8. RunInspector: PR via PrLine, no `Open PR ↗` · dn §8.

### F8 `project` · Project pane actions
Storyboard `06-flow-project.png`. Test: `headers.test.js`, `windows.test.js`.
1. Project pane header → gear (Project settings), bin (Remove project…, danger), `⋯` last · *06-flow-project* step 1 · R4.
2. `⋯` → Fetch, Shell, Diff main, Trust in a pane (untrusted projects only) · R4.
3. Worktree row → Diff icon button (± glyph, tooltip "Diff") and bin icon (danger, tooltip "Remove worktree…"), no text buttons · R2, R3.
4. Remove worktree → confirm dialog, then row disappears · dn §13.1.
5. Gear → Project settings window "Project settings · <name>" on its first section; one window at a time · dn §3.1.

### F9 `settings` · App settings window
Storyboard `06-flow-settings.png`. Test: `windows.test.js`, `floatwindow.test.js`, `settings-sections.test.js`, `terminalsettings.test.js`, `memorysettings.test.js`.
1. ⌘, (or gear) → floating window over the app, dimmed backdrop, 960 wide (944 at 1024), `role="dialog"`; ⌘, again closes · *04-settings-integrations-1280*, *02-floating-window* · dn §3.3.
2. Nav lists 9 sections with no `Kbd` and no Keyboard section; each renders its designed frame (no "Not designed yet") · *04-settings-\*-1280* · dn §13.2.
3. Integrations → cards, needs-you first; click a link to another section → window updates in place · *04-settings-integrations-1280*.
4. Deep link from a banner / toast / StatusBar PR chip → lands on its section with server or PR preselected, no new tab · dn §3.2.
5. Edit a key → input + Save + Cancel; Remove / Replace only on the row · *04-settings-integrations-editing-a-key-1280* · dn §8.
6. Esc → closes (an open Select consumes Esc first); focus returns to the opener · dn §3.3.
7. Section frames present with their state variants: Claude Code (update-available, update-failed), MCP servers (server-error), Terminal & editor (no-kittyconf; only Appearance + read-only Terminal + Open kitty.conf), Pacing policy (shadow-mode; Apply form), Memory (warn; bar from the sample, threshold default 1600 MB), Overwatch (Notifications switches), Permissions, Builds (no channel Select) · dn §13.2.
8. Autosave sections close without a prompt; sections with a Save/Revert footer ask "Discard changes?" · dn §3.3.
9. Keyboard focus is trapped; Tab cycles inside · dn §2.4.

### F10 `project-settings` · Project settings window
Storyboard `06-flow-project-settings.png`. Test: `windows.test.js`, `settings-sections.test.js`, `projectgeneral.test.js`.
1. Project pane gear → window "Project settings · orch", 7 sections, General first in the nav · *04-project-settings-files-1280* · dn §3.1.
2. General → rename + default branch Select are the only dirty fields; footer Save + Revert; path actions, fetch, trust, worktrees, danger zone; untrusted state shows Trust in a pane · *04-project-settings-general-1280*, *…-untrusted-1280* · spec §3.1 (Name, Default branch ship).
3. Task backend, Linear not connected → link opens Settings › Integrations, which **replaces** this window; Discard guard if dirty · *04-project-settings-task-backend-linear-not-connected-1280* · dn §3.2.
4. Files → access Select `read` / `read & write` per row; missing-file state · *04-project-settings-files-1280*, *…-missing-file-1280* · R11.
5. Each section saves itself: Save in Task backend does not save Notion (bug 1) · dn §9.
6. MCP overrides → Inherit / On / Off per server; deep link with a project opens this section with the server selected · *04-project-settings-mcp-overrides-1280* · dn §12.1.
7. Allocation → "Edit weekly plan…" opens the plan dialog above the window; Cancel only · *04-project-settings-allocation-1280* · dn §3.2.
8. Intake, Notion sections as drawn; Select height = input height · *04-project-settings-intake-1280*, *…-notion-1280* · R11.
9. Close → window gone, one window at a time · dn §3.1.

### F11 `planner` · Week planner
Storyboard `06-flow-planner.png`. Test: `windows.test.js`, `surfaces.test.js`.
1. Title-bar PacingIndicator → popover with details + **Week planner** button · *03-title-status-banner*.
2. Button (or leader g, banner `week`/`5h`/`red`/`ac`) → Week planner window 1100 wide (app − 80 at 1024) · *04-week-planner-1280*, *…-1024* · dn §3.2.
3. Plan a feature into a slot (Edit plan…, Defer / Keep running, Pause all, Weekly report, Shadow/Sprint mode, Governor log with Undo) · *04-week-planner-1280*.
4. Pause all gives the same toast from button and key · dn §9 bug 5.
5. Close → status bar has no planner entry (pressure and ☾ are information only) · dn §8.
6. Weekly report button opens the report; palette `report.list` opens the list · dn §9 bug 2.

### F12 `shortcuts` · Find a key
Storyboard `06-flow-shortcuts.png`. Test: `shortcuts.test.js`, `keyboard.test.js`, `hints.test.js`.
1. Hover a button → tooltip `Label · key` (e.g. `Resume · ⏎`), no pill · *02-tooltip* · dn §2.5.
2. ⌘/ (also on AZERTY, Help › Keyboard Shortcuts, palette) → Keyboard shortcuts window, two-column groups, `Leader = ⌘;` line · *04-keyboard-shortcuts-1280*, *…-1024* · dn §7, §3.4.
3. Search "dock" → rows filter by label or key, empty groups hide; "cmd" matches ⌘ · dn §7.
4. ⌘K → palette rows with bindings right-aligned · dn §7.
5. Empty pane → one muted line "⌘K to open anything" · *03-empty-workspace-project*.
6. Leader overlay shows `LEADER` only while ⌘; is held · *03-title-status-banner*.
7. No `.kbd` beside or inside any button/link outside the allowed places (menu items, palette, leader overlay, empty line, sheet) · *02-button-vs-hint*.

### F13 `terminal` · Shell and editor
Storyboard `06-flow-terminal.png`. Test: `shellcwd.test.js`, `terminalsettings.test.js`, `keyboard.test.js`.
1. New shell (leader t, dock `+` in Shells) → opens in the dock with cwd of the focused worktree; title `zsh · <cwd>` · *03-dock-shells* · dn §6.
2. Open a file from agent output → nvim opens as a workspace **tab** (project-bound), no split · dn §4.1.
3. Settings › Terminal & editor → Appearance: theme Select (`setTheme`) and kitty-colours switch (`setKittyTheme`); Terminal block read-only; missing kitty.conf shows its text · *04-settings-terminal-editor-1280*, *…-no-kittyconf-1280* · dn §13.4. Renderer, Editor command, Shells rows hidden (spec §3.1).
4. Open in kitty (agent `⋯`, project `⋯`/Shell, Terminal settings) → kitty opens in that worktree · dn §1.3.1.
5. Leader `v` `s` `h` `l` `z` do nothing (`j` `k` stay: into / out of the dock); no `pane.split.*` / `pane.zoom` in palette or menu · dn §4.1.

### F14 `migrate` · Restart and old layouts
Storyboard `06-flow-migrate.png`. Test: `persist.test.js`, `layout.test.js`, `windows.test.js`, `sidebar.test.js`.
1. Quit with context `orch`, dock Shells, a project collapsed, a feature expanded → layout saved with `workspace.project`, `dock.view`, `dock.agentProject`, `dock.activeAgent`/`activeShell`, `sidebarTree` · dn §5.4.
2. Relaunch → same context, same dock view and filter, same active tabs, same collapsed tree; unknown project → All; vanished project/feature keys pruned · dn §5.4.
3. `activeAgent` outside the filter → first agent tab of the filter, else empty view; `dock.view` absent → kind of `dock.active` · dn §12.4.
4. Old saved split layout → one tab strip in reading order, saved focused tab active; saved `settings` / `project-settings` / `planner` panes dropped, no window opens at launch · dn §4.2, §3.1.
5. Closed windows are not offered by ⌘⇧T · dn §3.1.
6. Not restored on purpose: windows, tab scroll, Select popovers · dn §5.4.

## Part 2 · Surfaces (audit §8, as amended by lead 2026-11-09)

Tick when the surface shows exactly these actions and nothing more.

### Title bar
- [ ] Jump search field-button (tooltip `… · ⌘K`) · [ ] PacingIndicator (popover: details + Week planner) · [ ] Decisions (count) · [ ] no pills · *03-title-status-banner*

### Banner
- [ ] Only app-wide kinds: `core` ×; `claude` Set up, ×; `week` / `5h` Week planner, ×; `red` Week planner, Pause all, ×; `ac` Week planner, ×; `w:<text>` ×
- [ ] No agent condition, no `Later`, no `Details`, no `Kbd` · *03-title-status-banner*

### Sidebar and icon rail
- [ ] `+` Add project (tooltip `Add project · ⌘N`) · [ ] tree rows project / feature / agent · [ ] carets on project and feature rows only, state remembered · [ ] no `⌘B` pill
- [ ] In All projects no project row or rail mark is highlighted · *03-app-1512*, *03-context-all-projects*

### Workspace tab strip
- [ ] Context Select (compact, hugs, max 220; counts, waiting dot) · [ ] tabs (icon, full title, × on hover/active) · [ ] `‹ ›` only when overflowing · [ ] no pane header · *02-tab-strip*, *03-tab-overflow-1280*

### Dock strip
- [ ] View Select (Agents n / Shells n = open dock tabs, all projects; waiting dot per dn §12.7) · [ ] project Select (Agents view) · [ ] tabs · [ ] `‹ ›` · [ ] `+` (Shells view, not when empty) · [ ] Hide dock icon · *02-dock-strip*, *03-dock-agents-filtered*, *03-dock-shells*

### Status bar
- [ ] Information only: connection, pressure, ☾, agents/queue counts · [ ] PR build and update chips only when present · [ ] `LEADER` only while held · [ ] pressure and ☾ do not open the planner · *03-title-status-banner*

### Agent pane
- [ ] Header: role mark, id, StatusBadge, model, ctx, session, WaitingChip (only without strip), UndeliveredChip · [ ] Interrupt + Suspend (live only) · [ ] `⋯` last
- [ ] Strips: permission, needs you, queued, blocked by limit, error, suspended, each per live / not live · [ ] watch bar · *04-agent-strips*, *04-agent-waiting*, *04-agent-suspended*

### Agent inspector
- [ ] Details · [ ] Open session · [ ] `⋯` = same AgentMenu · [ ] no Resume / Interrupt / Suspend · *04-agent-inspector*

### Run inspector
- [ ] Blocker link · [ ] members · [ ] PrLine (no `Open PR ↗`) · [ ] Open feature · [ ] Open board

### Project pane
- [ ] Header: gear, bin (danger), `⋯` = Fetch, Shell, Diff main, Trust in a pane (untrusted) · [ ] sessions · [ ] worktrees: Diff ± icon, Remove… bin icon, tooltips · [ ] PRs (Load) · R2–R4

### Feature pane
- [ ] Header icons Pause/Resume, Stop, Delete (danger); `⋯` = Board, Canvas, Publish/Refresh in Notion, Open in Notion · [ ] docs tabs + repo path line + Open in nvim icon · [ ] PrLine (link, CI dot + count, checks popover) · [ ] team rows · [ ] timeline chips · [ ] message box (To Select, input, Send) · [ ] no Board button in Tasks card · *04-feature-pane*, *04-feature-pr-running*, *04-feature-pr-review-done*

### Board
- [ ] Feature chips · Sync · New task · [ ] card menu: Open feature, Run off-hours, Edit, Open remote, Move to, Assign, keys right-aligned · [ ] no footer legend · *04-board-pane*

### Mission control
- [ ] No Filter button, no toolbar legend · [ ] follows the context (header text; empty state + Show all projects) · [ ] Ready to start lane follows the context · [ ] rows with their own buttons; Ready row buttons carry tooltips, no pills · *04-mission-control-context*

### Team canvas, Diff, Report, Setup, dialogs (no frames)
- [ ] No legends or key pills · [ ] native selects → Select · [ ] Connect/Delete as inspector buttons · [ ] Duplicate button with key `⇧D` in tooltip and sheet · [ ] dialogs in the FloatingWindow shell, Cancel/submit with tooltips, no pills · [ ] status icons use StatusBadge vocabulary · [ ] design preview has a visible ×

### Settings, Project settings, Week planner, Keyboard shortcuts
- [ ] All four are floating windows (never tabs; no `addPane`) · [ ] Settings 9 sections, Project settings 7, every one as its frame · [ ] hidden rows (top of this file) absent · [ ] Select height = input height, compact only in the two strips · [ ] 34 native selects gone (`noselect.test.js`) · [ ] one model list · [ ] Revert label on every footer · [ ] Discard-changes guard on dirty footers · *02-floating-window*, *04-settings-\**, *04-project-settings-\**, *04-week-planner-\**, *04-keyboard-shortcuts-\**

### Whole app
- [ ] `grep -rn "<select" ui/src` empty · [ ] `<Kbd` only in allowed places (`hints.test.js`) · [ ] `make test`, `make lint` pass · [ ] design-notes §9 bugs 1–7 fixed
