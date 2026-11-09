# Improve Orchestrator UI UX · App audit (T-288)

> Companion of [spec.md](spec.md) §3.1–§3.2 and [ux-brief.md](ux-brief.md). Read from `ui/src` at the integration branch's base (main 5882f8b). Paths are relative to `ui/src/`; `app.js` = `lib/app.svelte.js`. This file is the input of T-289/T-290 (what to draw) and T-291 (design notes).

## 0. Findings that change the brief

1. **The app banner never shows an agent state.** Every banner kind is app-wide (§3). The "suspended agent: banner + header" duplicate the user saw is the agent pane's **strip** plus its **header** (`panes/AgentPane.svelte` :139 and :183/:192), with the Inspector repeating it a third time (`inspector/AgentInspector.svelte:49`). Rule 1 of the brief holds as is (nothing moves out of the banner); the dedup happens in the agent pane and the inspector.
2. **Hints point at keys that do nothing**: Banner `o` / `esc` (Banner.svelte:44-45, no handler), AgentInspector `⏎ esc s r ⇧K ⌘; t` (no key handler in inspector files), AgentPane header `esc interrupt` (:136, Esc in the pane goes to Claude Code or leaves watch mode), queued strip `⏎` (:171, no handler), Dialogs `⌘,` (:323, chords are dropped while a dialog is open). All go away with the hint rule.
3. **Behaviour bugs found on the way** (for the implementation feature, §9): Project settings' two Save buttons save both sections; `report.list` opens the current report; Board menu "Open task" opens the feature; Allocation "Edit the plan in the Week planner" opens the plan dialog, not the planner; Planner Pause all gives different feedback by button and by key; Team canvas Duplicate is `D 1/2` on buttons and `⇧D` in the legend; four model lists with different contents.

## 1. Rules applied (from ux-brief)

R1 Banner = app-wide only · R2 pane header = identity + pane-wide actions, strip = the action that resolves the state, never both · R3 Inspector = details + secondary actions, never the pane's primary action · R4 toolbar = frequent, `⋯` Menu = the rest, never both · R5 status bar = information only (§2.3; updated in ux-brief) · R6 the palette is the catalogue, not a duplicate.
H (hints): no `Kbd` inside or next to a button; the key goes in the tooltip (`Label · key`). Kept: palette rows, Menu items (right-aligned), Keyboard shortcuts sheet, one muted line in an empty state, the leader overlay, text-field placeholders (`Message the Overwatch · ⏎ to send`).

## 2. Duplicates and their single home

### 2.1 Agent (the case of the brief)

| Action | Places today | Single home | Removed from |
|---|---|---|---|
| Resume | AgentPane header `⏎ resume` :139 (every not-live state); strip Resume :183 (error), :192 (suspended); AgentInspector Resume :49; palette `agent.resume` | **Strip**, for every not-live agent whatever the strip: error, suspended, permission, needs you add a **Resume** button; queued has **Run now** instead; blocked by limit shows Resume **disabled** (tooltip "Blocked until the reset · hh:mm") | header :139, AgentInspector :49 |
| Kill | suspended strip :193; AgentInspector :51; palette `agent.kill`; Mission ⇧K | **Header `⋯` menu** (any state); the AgentInspector shows the same `⋯` menu (below) | strip :193, AgentInspector buttons |
| Delete… | error strip :184, suspended strip :194; AgentInspector :52; palette | **Header `⋯` menu** (+ inspector `⋯`) | strips, AgentInspector buttons |
| Interrupt, Suspend | header :136-137 (live); AgentInspector :46-47 | **Header** (live only), as icon+label buttons | AgentInspector |
| More (`⌘K more` opens the palette) | header :141 | replaced by the header **`⋯` Menu**: Kill…, Restart fresh…, Delete…, Shell in its worktree, Diff against base, Open in kitty, Run off-hours / in working hours | — |
| Shell here, Diff, Open in kitty | AgentInspector :53-55; palette | **Header `⋯` menu** (+ inspector `⋯`) | AgentInspector buttons |
| Open session | AgentInspector :44 | **AgentInspector**: **Open session** + the same `⋯` menu as the pane header (R3: secondary actions; the inspector shows an agent selected in the chat that may not be open, so Kill/Delete/Diff/Shell must stay reachable there). Interrupt / Suspend / Resume stay out of it (primary, in the pane). Accepted asymmetry with Kill/Delete: resuming means looking at the session, so it goes through **Open session** (one click, the strip's Resume is then right there) or the palette; Kill/Delete/Diff/Shell don't need the session | — |
| WaitingChip link | header :124 and strip :162/:166/:176 | **Strip** when a strip shows, else header | header while a strip shows |
| Status label | StatusBadge :123 and strip label :162/:165 | **Header** StatusBadge | strip repeats only the reason, not the label |
| Permission decision (Allow once / Deny / Always allow / Answer in the pane) | perm strip :150-156; the decision card in the chat column | **Both kept (accepted)**: the strip acts where the agent is, the card is the decision inbox. The strip drops the `⌘⇧I` text and says "also in Decisions" as a link | — |

### 2.2 Shell, workspace, dock

| Action | Places | Single home | Removed from |
|---|---|---|---|
| Zoom / restore | PaneFrame :38, Dock :126, leader z | **removed** (one pane, spec §3.4) | all |
| Close pane | PaneFrame `⌘W close` :39, Dock tab × :116, Node tab × :64 | **Tab ×** (workspace tab strip, dock tab) | PaneFrame header (PaneFrame goes) |
| Pane number badge | PaneFrame :33 | **removed** (⌘1–9 = tabs of the context) | — |
| Split handle, splits | Node :45-53, leader v/s | **removed** | — |
| New session | Workspace empty :19, ProjectPane :162, MissionPane :325, ⌘T | each in its own empty state (accepted: never two on one screen) | pills removed |
| Add project | Sidebar + :44, Sidebar empty :99, MissionPane :313, SetupPane :91, ⌘N | Sidebar `+` (with projects); the empty states keep theirs (only one shows) | pills removed |
| Hide dock | hint `⌘J hide` :128 (not a button) | **icon button** at the dock strip's right end, tooltip `Hide dock · ⌘J` | hint |

### 2.3 Title bar, status bar, banner, Week planner entries

The Week planner opens from 10 places today: Banner :42 and :44, StatusBar :35 (pressure pill) and :41 (☾ chip), PacingIndicator :72, Settings › Pacing :147, :203 and `p`, ReportPane Esc, palette/leader g.

| Action | Single home | Removed from / changed |
|---|---|---|
| Open Week planner (usage) | **Title-bar PacingIndicator**: a click opens the popover; the popover's **Week planner** button opens the window | StatusBar pressure pill :35 and ☾ chip :41 become plain information (R5; they show state, the title bar is the entry) |
| Open Week planner (from a limit banner) | Banner button **Week planner** (week, 5h, red, ac) | `Details` :44 renamed **Week planner** (same action, one label) |
| Dismiss banner | **×** :48 | `Later` :45 (same effect) |
| Pause all | Banner red :46 + palette (R6) | — (Planner `P` button stays in the planner window: a different screen) |
| Settings › Pacing → planner | one link in the section intro :147 | :203 link, `p` key |
| Decisions | TitleBar Decisions :21 (+ Inspector overlay banner :22 when the chat is hidden: accepted, the overlay covers the title bar's target) | DecisionInspector "Show card" :24 stays (it jumps to that card, not the list) |
| Updates / PR builds | StatusBar chips :46, :51 (they appear only when there is something) | — |

### 2.4 Panes

| Pane | Duplicate | Decision |
|---|---|---|
| FeaturePane | Board: header `Board b` :249 and Tasks card `Open board b` :282 | **Header**; the Tasks card loses its button (its title stays) |
| ProjectPane | Diff in header :135 (main checkout) and per worktree :177 | **Both kept** (different targets); header label becomes "Diff main" |
| ProjectPane | Remove project… :137 then panel Remove :223 | one flow, kept |
| BoardPane | footer legend repeats the menu's keys :317 vs :296-301 | **Menu** keeps its right-aligned keys; footer legend removed |
| BoardPane | "Open task ⏎" opens the feature :296 | renamed **Open feature** |
| MissionPane | Filter button + toolbar hint + empty-state hint; ProjectFilter popover | **removed**: Mission control follows the **workspace project context** (spec §3.4). Empty state: "Nothing in <project> · Show all projects" (sets the context to All). Palette `mission.filter` sets the context |
| MissionPane | runNow / cancelQueued / resume / focusCard / PR: row buttons + toolbar legend + keys | **Row buttons** (QueueRow, "Show card", RunRow PR link); toolbar legend :299 removed (keys in the sheet) |
| TeamCanvasPane | Connect/Delete buttons + legend :559 | **Inspector buttons**; legend removed |
| TeamCanvasPane | Launch header :506 / library :600; New team :565 / :607 | kept: different targets / different states |
| RunInspector | PR link :54 and `Open PR ↗` :66 | **PR link** (PrLine); `Open PR ↗` removed |
| DesignSection | key legends :207, :229 | removed; the preview overlay gets a visible **×** close |

### 2.5 Settings, Project settings, Week planner

| Duplicate | Decision |
|---|---|
| Project settings Save at Task backend :293 and Notion :312, one `save()` for both | **Each section saves itself** (bug for implementation); one Save + Cancel per section, at the section's foot |
| Integrations row Remove :163 and SecretField Remove :67; row Replace :162 and SecretField Replace :65 | **Row buttons**; SecretField keeps only its input + Save / Cancel |
| MCP Reconnect: row link :268 and drawer hint button :356 | **Row link**; the drawer shows the hint text only |
| MCP registry for a project: Settings › MCP servers with projectId (palette :1264, RoleMcp :35/:48, Dialogs :323, decision cards via `routeOpen` :466) and Project settings › MCP overrides :314 | **Project settings › MCP overrides**: every deep link with a projectId opens the Project settings window on `mcp`; Settings › MCP servers is the global registry only |
| Allocation: "Set up the weekly plan" :240 / "Edit the plan in the Week planner" :243; "Keep in the plan" :222 vs Cancel :248 (both esc) | one **Edit weekly plan…** (opens the plan dialog, label fixed); **Cancel** only |
| Overwatch on/off: palette + ChatInput hint link + section switch | switch in Settings › Overwatch; the others are entries (R6, kept) |
| Weekly report: Planner button + 2 palette entries running the same | Planner button; `report.list` must open the list (bug) |

### 2.6 Components with actions but no hint and no duplicate

Checked, nothing to change beyond using the shared Select/Menu/tooltip: IconRail (project mark → `openAgent`, hover popover agent rows: the narrow form of the sidebar), PermissionCall (Show n more lines / Show less), OutcomeRow (row select, link), ChatText (reply jump, Show more, Retry, Remove, look at its session), CodeBlock (Copy on hover), MemberChip (chip → `openSession`, UndeliveredChip), blocks/* (DiffSummary Show more files / Show patch, Docs file link, Error Show detail, LastMessage Show more, Links jump, Markdown source ↗, Options radio/checkbox), Markdown (ref / external / anchor / footnote links), MdImage (chip ↗, open large; its overlay gets a × per §4), WaitingChip, UndeliveredChip, LifecycleMarker, PrLine (PR link, CI popover, refresh). No Kbd in any of them.

## 3. Banner kinds (all stay: app-wide)

| id | Shown when | Actions after cleanup |
|---|---|---|
| `core` | no agent core (`d.core.error`) | × |
| `claude` | Claude Code problem (read-only mode) | Set up, × |
| `week` | weekly limit reached | Week planner, × |
| `5h` | 5-hour limit reached | Week planner, × |
| `red` | Governor Red, not shadow | Week planner, Pause all, × |
| `ac` | off-hours work pending without AC power | Week planner (was Details), × (Later removed) |
| `w:<text>` | core/runtime warnings | × |

Banner.svelte:10-48. Dismissals stay in memory (unchanged). No banner button carries a `Kbd` any more.

## 4. Inline hints: decisions

Every inline hint is **removed** from the chrome; the key moves to the control's tooltip as `Label · key` when the control exists, else only to the Keyboard shortcuts sheet (§7). Kept (allowed by H): palette rows and input (`Palette.svelte:43,:53`), Menu items (Board card menu, the new `⋯` menus), the empty-state one-liner, the leader overlay (StatusBar `LEADER` :54 → shown only while the leader is held), text-field placeholders.

| Where | Hints today (file:line) | After |
|---|---|---|
| TitleBar | `⌘K` :18, `⌘⇧I` :24 | tooltips "Jump or run a command · ⌘K", "Decisions · ⌘⇧I" |
| Banner | `⌘; g` :42, `o` :44, `esc` :45 | none |
| StatusBar | `⌘;` leader :54 | shown only while the leader is held |
| Sidebar | `⌘B` :44, `⌘N` :99, title "(⌘N)" | tooltips "Add project · ⌘N"; ⌘B in the sheet |
| Dock | `⌘; z restore` :126, `⌘J hide` :128 | zoom gone; Hide dock icon tooltip |
| PaneFrame | `⌘; z`, `⌘W` :38-39 | PaneFrame header gone; tab × tooltip "Close · ⌘W" |
| Workspace empty | `⌘T` :19, `⌘; o / g / t` :20 | one line "⌘K to open anything" + two buttons (ux-brief): **Open <project>** and **New session** in a project context, **Mission control** and **New session** in All projects (tooltip ⌘T on New session) |
| Dialogs | `esc` on every Cancel, `⌘⏎` / `⏎` on every submit, `⌘,` :323 | none in buttons; tooltips "Cancel · esc", "Start session · ⌘⏎" |
| Toasts | `u` :12 | tooltip "Undo · u" |
| Inspector | `⌘⇧I` :22, `esc` as the close button's only content :23, `⌘I` :23 | × icon button "Close · esc"; others tooltips |
| AgentInspector | `⏎ esc s r ⇧K ⌘; t` :44-53 (no handlers) | none (only Open session left) |
| DecisionInspector / RunInspector | `d` :24, `· Show card d` :33, `o` :66 | none |
| PacingIndicator | `⌘; g` :72 | tooltip "Week planner · ⌘; g" |
| ChatColumn | `esc` replacing × :860, `t` :900/:1057, `f` :910, `⏎ j` :985, `c` :1043 | × always (tooltip "Clear · esc"); others in the sheet |
| ChatInput | line `⏎ send · ⇧⏎ new line · esc leave / ⌘L` :73 | placeholder "Message the Overwatch · ⏎ to send" |
| ChatLine / DecisionCard | `u` :102/:123/:231, `⌘⏎` `esc` :119-120, `x.key` :214/:235, `o` :234, reply hint line :181 | tooltips "Approve · a" etc.; reply box placeholder "Reply · ⏎ to send" |
| AgentPane | watch bar `⏎ take over`, `esc back` :108-110; over-Mission bar `⌘; o` :116; header `esc` `⏎` `⌘K` :136-141; strips `⏎` :171/:183/:192, text `⌘⇧I` :148/:162 | watch bar keeps **two buttons** "Take over" / "Back to Mission control" (tooltips ⏎ / esc); over-Mission bar: a "Mission control" link; others tooltips |
| ProjectPane | `f x t ⌘T p` :133-198 | tooltips |
| FeaturePane | `N b c p e b` :247-282, `⏎` :341, text `(⌘,)` :105, `(d)` :353 | tooltips; message box gets a **Send** icon button (tooltip "Send · ⏎") |
| DesignSection | `d` :169, `· x` :200, `⏎` :201, legends :207, :224, :229 | tooltips; preview gets × |
| BoardPane | `r n` :230-231, placeholders `⌘⏎ to create/save` :238/:257, footer legend :317 | tooltips; placeholders kept; menu keeps keys |
| MissionPane / QueueRow / ProjectFilter | `f` :292, toolbar legend :299, `d` :276, `⌘N ⌘T` :313/:325, `f` :318, `⏎` :344, `n r` QueueRow :27/:30, filter footer :60 | tooltips; legend removed; ProjectFilter removed |
| DiffPane | `r c` :97-98 | tooltips |
| ReportPane | `e ⏎` :109-110, `1–4` :156, `R [ ]` :227 | tooltips |
| TeamCanvasPane | `⇧R ⌘⏎` :503/:506, legend :559, `(⇧R)` :564, `esc cancel` :573…623, `1–4` `1–9` :576/:583, `⏎ g` :613-614, `D 1/2` :621-622, `c x` :677-678 | tooltips; the connect/add menus are Menus (keys right-aligned); legend removed |
| SetupPane / PlanEditor / MdImage | `⌘N` :91, `(e)` :130, `esc` :97, "esc or click to close" :67 | tooltips; MdImage overlay gets × |
| Settings (all sections) | nav `⌘,` :91 and `j k · ⏎` :100; placeholder `⌘K` :119; every section's foot legend (Integrations :174, MCP :275, Pacing :152/:220, Builds :122/:159, Files :334, Allocation :251, Project settings :210/:296); pills in Test/Connect/Remove/Save/Cancel/Retry/Add/New server/Apply… (see T-288 research: IntegrationsSettings :161-163, McpSettings :248/:296/:366-370, PacingSettings :151/:216, BuildsSettings :143/:154, FilesSettings :248/:281/:306-307, AllocationSettings :171-248, ProjectSettingsPane :228-312, SecretField :51-56) | none inline; tooltips; the window title bar has × "Close · esc"; Keyboard sheet has a Settings group |
| PlannerPane | `S P R e` :210-213, `O` :310, `a r` :320-321, `u` :336 | tooltips |

## 5. Settings-like screens → floating windows

| Window | Today | Sections (nav) | Notes |
|---|---|---|---|
| **Settings** (⌘,) | `openSettings` (`lib/app.svelte.js`) → `addPane(type settings)` | Claude Code*, Integrations, Overwatch, Permissions, MCP servers, Terminal & editor*, Pacing policy, Memory*, Builds (*placeholders today: kept as "Not designed yet", out of scope). **Keyboard removed** (→ Keyboard shortcuts sheet) | ⌘, on an open window focuses it; a deep link switches the section. Openers: ⌘,, palette `settings*`, `overwatch.settings`, `update.pr`, toast "Restart to run" :1149, StatusBar PR chip :46, ChatInput link :69, IntegrationsSettings :111, Project settings links :237/:311, decision cards `routeOpen` :466 |
| **Project settings · <project>** | `openProjectSettings` (`lib/app.svelte.js`) | General*, Files, Task backend, Allocation, Notion, MCP overrides | Openers: ProjectPane Settings :136, palette `project.settings.*`, and every MCP deep link with a projectId (§2.5) |
| **Week planner** | `openSingleton('planner')` (`lib/app.svelte.js`) | no nav; current content (forecast, allocations, next action, log, Shadow/Enforce, Sprint, Pause all, Weekly report, Edit plan) | Openers after §2.3: PacingIndicator popover, banners, Settings › Pacing link, ReportPane Esc, palette/leader g |
| **Keyboard shortcuts** (⌘/, new) | Settings › Keyboard placeholder | groups of §7, search field | — |

The Weekly report, Setup, Mission control, Team canvas stay workspace tabs. The PlanEditor and the other dialogs stay dialogs (Dialogs.svelte), drawn with the new floating-window shell at their current small sizes.

## 6. Single pane: commands and keys removed

| Command id | Key | Where |
|---|---|---|
| `pane.split.right` | leader v | keyboard.js `LEADER`, `commands()` |
| `pane.split.below` | leader s | `commands()` |
| `pane.zoom` | leader z | `commands()`; `toggleZoom`, `app.zoomed` |
| `focus.left` / `focus.right` | leader h / l | `run` map in `commands()` |
| `pane.focus.1…9` | ⌘1…9 | **kept, re-meant**: tab n of the current context (`focusNumber` in `lib/app.svelte.js`) |
| `focus.down` / `focus.up` | leader j / k | **kept**: into / out of the dock |

Components that go: PaneFrame's header (number, kind, zoom, close; PaneFrame.svelte:31-39), Node split handles (Node.svelte:45-53). Workspace renders one tab strip + the active pane.

## 7. Keyboard shortcuts sheet (⌘/)

Real bindings only (keyboard.js CHORDS :6-20, LEADER :26-42, pane key handlers). Leader = ⌘;.

- **General**: ⌘K Jump or run a command · ⌘P Jump · ⌘, Settings · ⌘/ Keyboard shortcuts · ⌘B Sidebar · ⌘I Inspector · ⌘⇧I Decisions · ⌘L Ask the Overwatch
- **Workspace**: ⌘T New session · ⌘N Add project · ⌘W Close tab · ⌘⇧T Reopen closed tab · ⌘1–9 Tab 1–9 · leader o Mission control · leader g Week planner · leader e Editor (nvim) · leader w Next waiting agent · leader p Pause / unpause agent
- **Dock**: ⌘J Show / hide · leader t New shell · leader ] / [ Next / previous tab · leader j / k Into / out of the dock
- **Agent pane**: ⏎ Resume (on the strip) · watch mode: ⏎ Take over, esc Back to Mission control
- **Decisions & chat**: / Search · j k Move · ⏎ Open · a Approve · r Reject · e Edit · x Dismiss · u Undo · o Open link · t Today · [ ] Previous / next day · f Filter · c Clear digest · esc Leave
- **Mission control**: j k Rows · h l Chips · ⏎ Open / show card · space Details · d Decision · o Open / PR · n Run now · ⌫ Cancel queued · i Interrupt · p Pause · s Suspend · r Resume · ⇧K Kill
- **Board**: h l Columns · j k Cards · ⇧h ⇧l Move card · ⏎ Open feature · space Menu · n New task · e Edit · a Assign · m Move to · 1–5 Column · g Open remote · ⇧O Off-hours · r Sync
- **Feature**: e Open doc in nvim · b Board · c Canvas · p Pause / resume team · m Message · d Design · N Publish to Notion
- **Project**: f Fetch · p Load PRs · t Trust · x Remove project
- **Diff**: j k Files · r Refresh · c Create PR · e Open in editor
- **Weekly report**: 1–4 Breakdown · R Past reports · [ ] Previous / next · e Copy as markdown · ⏎ Plan next week · esc Week planner
- **Team canvas**: n Add role · c Connect · ⏎ Edit · = Fit · x Delete · u Undo · ⇧D Duplicate · ⌃R Redo · ⇧R Library · ⌘⏎ Launch on feature
- **Design preview**: h j k l Move · ⏎ Open · space Preview · x Ask to export · ← → Step · 0 / 1 Fit / 100 % · esc Close
- **Week planner**: S Sprint · P Pause all · R Weekly report · e Edit plan · O Off-hours · a Apply · r Keep running · u Undo · j k Log
- **Settings lists**: j k Rows · ⏎ Edit / connect · t Test · x Remove · n New server · a Add · ⌫ Remove · ⌘O Browse · r Retry / sync · ⌘⏎ Save · esc Cancel / back to sections

## 8. What UI draws: actions per screen after cleanup

- **Title bar**: Jump search field-button · PacingIndicator (popover: details + Week planner) · Decisions (count). No pills.
- **Banner**: text + its buttons per §3 + ×.
- **Sidebar**: `+` Add project · tree rows (project, feature, agent); carets on project and feature rows only, agent rows are leaves (state remembered). No `⌘B` pill.
- **Sidebar / icon rail in All projects**: no project row highlighted (the context is not a project).
- **Workspace tab strip**: project context Select · tabs (icon, full title, × on hover/active) · ‹ › when overflowing. No pane header.
- **Dock strip**: view Select (Agents n / Shells n = the number of agent / shell **tabs open in the dock**, all projects; not every agent of the sidebar; waiting dot) · project Select (Agents) · tabs · ‹ › · `+` (Shells view) · Hide dock icon.
- **Status bar** (information only): connection, pressure, ☾ off-hours, agents/queue counts; PR build and update chips (clickable, only when present); `LEADER` only while held.
- **Agent pane**: header = role mark, id, StatusBadge, model, ctx, session, WaitingChip (when no strip), UndeliveredChip; buttons Interrupt + Suspend (live) · `⋯` Menu. Strips: permission (Allow once, Deny, Always allow, Answer in the pane, "also in Decisions" link; when not live: Resume replaces Answer in the pane, which needs a live session) · needs you (WaitingChip; + Resume when not live) · queued (Run now + cost, Cancel) · blocked by limit (text + Resume disabled with the reset time) · error (Resume) · suspended (Resume). Watch bar: Take over · Back to Mission control.
- **Agent inspector**: details + Open session + `⋯` menu (Kill…, Restart fresh…, Delete…, Shell in its worktree, Diff against base, Open in kitty, Run off-hours).
- **Run inspector**: blocker link, members, PR line, Open feature, Open board.
- **Project pane** (R2–R4): header gear (Project settings) · bin (Remove project…, danger) · `⋯` = Fetch, Shell, Diff main, Trust in a pane (untrusted); sessions; worktrees (Diff ± icon, Remove… bin icon, tooltips); PRs (Load).
- **Feature pane** (R9, R10, R13): header icons Pause/Resume, Stop, Delete (danger) · `⋯` = Board, Canvas, Publish/Refresh in Notion, Open in Notion; docs (tabs, repo path line with Open in nvim icon); PR line (link, CI dot and count, checks popover); team rows; timeline chips; message box (To Select, input, Send).
- **Board**: feature chips · Sync · New task; card menu (Open feature, Off-hours, Edit, Open remote, Move to, Assign) with keys; no footer legend.
- **Mission control**: no Filter button (follows the context); rows with their buttons; no toolbar legend.
- **Team canvas, Diff, Report, Setup, dialogs**: no frames (T-290 decision): as today minus legends and pills (§4), native selects → Select (§9), tooltips per §4. Status icons everywhere use the StatusBadge vocabulary (the dock tabs included).
- **Settings / Project settings / Week planner / Keyboard shortcuts**: floating windows (§5).

## 9. Native `<select>` to replace with Select (27)

| file:line | selects | options |
|---|---|---|
| components/AllocationSettings.svelte:195 | allocation tier | 4 static |
| components/McpSettings.svelte:306 | design tool of a server | 4 static |
| components/RoleMcp.svelte:63 | downgrade target model | 3 static |
| components/RoleMcp.svelte:67 | downgrade trigger | 3 static |
| components/PermissionsSettings.svelte:134 (one element, rendered for team and solo) | default effect | 2 static |
| components/ModelSelect.svelte:24 (used ×2, TeamCanvasPane:647-648) | model | 6: default + 4 aliases + custom… |
| components/OverwatchSettings.svelte:151 | Overwatch model | 3–4 |
| components/PlanEditor.svelte:85 (per row) | tier | 4 static |
| components/Dialogs.svelte:239 | new session: project | dynamic |
| components/Dialogs.svelte:244 | new session: role | dynamic |
| components/Dialogs.svelte:259 | new session: model | 4: default + 3 models |
| components/Dialogs.svelte:264 | new session: effort | 4 static |
| components/Dialogs.svelte:269 | new session: permission mode | 3 static |
| components/Dialogs.svelte:294 | launch: project | dynamic |
| components/Dialogs.svelte:299 | launch: team (invalid disabled) | dynamic |
| panes/ProjectSettingsPane.svelte:243 | Linear team | dynamic (search) |
| panes/ProjectSettingsPane.svelte:249 | Linear project | dynamic (search) |
| panes/ProjectSettingsPane.svelte:265 (per row) | board column for a Linear state | 5 static |
| panes/BoardPane.svelte:239 | new task: feature | dynamic |
| panes/BoardPane.svelte:244 | new task: node | dynamic |
| panes/BoardPane.svelte:265 | assign task to node | dynamic |
| panes/FeaturePane.svelte:337 | message recipient | dynamic |
| panes/TeamCanvasPane.svelte:642 | node role | dynamic |
| panes/TeamCanvasPane.svelte:651 | role effort | 4 static |
| panes/TeamCanvasPane.svelte:662 | permission profile | 3 static |
| panes/TeamCanvasPane.svelte:669 | budget priority | 4 static |
| panes/TeamCanvasPane.svelte:699 | review mode | 3 static |

Model lists differ (models only, not counting default/custom): Dialogs :259 (3, no fable), ModelSelect (4 with fable), OverwatchSettings (3), RoleMcp (3). The implementation should share one list.

## 10. Round 2 additions (user review of the PR)

- **Pane header pattern** (every pane): identity left, icon buttons (with tooltips) right, `⋯` Menu last. The `⋯` holds the rest (rule 4: never both).
- **Notification toggles** (`notify.desktop`, `notify.sound`): home = Settings › Overwatch › Notifications (switches); the palette entries stay (catalogue).
- **Theme and kitty colours**: home = Settings › Terminal & editor › Appearance; palette `theme.*` stay.
- **Renderer**: home = Terminal & editor; Memory links to it, no second control. **Live cap**: home = Pacing policy; Memory links to it.
- **Project name, default branch, fetch, trust, remove**: edited in Project settings › General; the project pane keeps the status (read-only card) and the header icons; Trust in a pane stays in the pane's `⋯` and in General (rule 2: the pane = the action resolving the state, General = the record).
- Settings and Project settings: every section has its frame (spec §7 R14); no `Kbd` in the nav.
