# Improve Orchestrator UI UX · Design notes (implementation hand-off)

> For the follow-up implementation feature (frontend). Companions: [spec.md](spec.md) (problem, scope), [ux-brief.md](ux-brief.md) (rules), [audit.md](audit.md) (every action and hint, file:line at main 5882f8b), [api-contract.md](api-contract.md) (state keys), and the PNGs in [design/](design/) (source: `Orchestrator.figcraft` on Google Drive, ADR-012). Paths are relative to `ui/src/` unless they start with `crates/` or `ui/test/`; `app.js` = `lib/app.svelte.js`. Line numbers are from the audit's base and drift: anchor on the function name.
>
> Where a PNG and this file disagree on a number or a label, the PNG wins for looks (spacing, colours, sample data), this file wins for behaviour. Sample data in the frames (project names, counts) is illustrative.

## 0. What changes, in one page

1. **Chrome**: no `Kbd` pill next to or inside a control. Keys live in tooltips (`Label · key`), the palette, Menu items, the Keyboard shortcuts sheet, one empty-state line, and the leader overlay (§7).
2. **One home per action** on a screen (§8): the agent pane strip owns Resume; the header `⋯` Menu owns Kill/Delete/Shell/Diff/Kitty; the title-bar pacing indicator is the Week planner's entry; Mission control follows the project context.
3. **Settings, Project settings, Week planner, Keyboard shortcuts** are in-app floating windows, never workspace tabs (§3).
4. **Single pane, project context**: no splits, no zoom, no pane numbers; one tab strip with a project Select, tabs that never shrink, `‹ ›` overflow (§4, §5).
5. **Dock = Agents / Shells views** with a project filter on Agents; shells titled by their live directory (§5, §6).
6. **Select** and **Menu** replace the 27 native `<select>` and every ad-hoc popover menu (§2).
7. **Remembered**: sidebar tree, workspace project, dock view and agent filter (§5.4).

New shared components (all in `components/`): `Select.svelte`, `Menu.svelte`, `Tooltip.svelte` (action `use:tooltip`), `TabStrip.svelte`, `FloatingWindow.svelte`, `ShortcutsSheet.svelte`. Deleted: `Node.svelte`, `PaneFrame.svelte` (header), `mission/ProjectFilter.svelte`, `SettingsPane.svelte`/`ProjectSettingsPane.svelte`/`PlannerPane.svelte` as *panes* (their bodies move into windows, §3). `Kbd.svelte` stays (tooltips, palette, menus, sheet, leader overlay) with no hover state and `pointer-events: none`.

---

## 1. Frame index (PNG → what changes)

PNG sizes: 1280 × 800 app frames unless the name says 1512 or 1024. "Today" = main 5882f8b. State names are the labels drawn in the frame.

### 1.1 `02-*` Components

| PNG | Component | Implementation |
|---|---|---|
| `02-select.png` | Select | §2.1. Frame shows: closed (default, hover, focused, open, disabled, error), compact, status dot, count, placeholder, open with groups + selected ✓ + highlighted + disabled option with tooltip reason, search (> 8 options, typing filters, "No project matches" line), flipped above. |
| `02-menu.png` | Menu | §2.2. Items: icon, label, right-aligned key, separator, destructive, destructive highlighted, disabled with reason, submenu (›), checkable (✓). Trigger `⋯` states default/hover/focused/open/disabled; tooltip "More actions" has no key. |
| `02-tab-strip.png` | TabStrip (workspace) | §2.3 + `components/Workspace.svelte` rewritten. Tab states default/hover/active/focused/active+focused/long title (max 280 then ellipsis); kind icons; strips: fits (no `‹ ›`), overflow at start, scrolled, at end, All projects context; arrow button states. |
| `02-dock-strip.png` | Dock strip | `components/Dock.svelte` (§5.2): view Select, project Select, tabs (agent, shell live cwd, long path left-truncated, command shell fixed title), right end `‹ ›` `+` Hide dock; empty views; overflow with 10 agents. |
| `02-floating-window.png` | FloatingWindow | §2.4 + §3. Settings / Week planner / Keyboard shortcuts title bars, nav item states, close × states, narrow 1024 note. |
| `02-button-vs-hint.png` | Buttons vs Kbd | `styles/` + `components/Kbd.svelte`: levels primary / secondary / ghost / danger / icon with default, hover, pressed, focused, disabled; `Kbd` and the one-line hint are static. The Do/Don't block is the review checklist for §7. |
| `02-tooltip.png` | Tooltip | §2.5: `Label`, `Label · key`, leader sequence, two lines (title + full value), disabled reason; below by default, flipped above at the window bottom. |

### 1.2 `03-*` Shell and layouts

| PNG | Anchors | States |
|---|---|---|
| `03-app-1280.png`, `03-app-1512.png` | `App.svelte` (no layout change), `components/Workspace.svelte` (tab strip + the active pane), `components/Dock.svelte`, `components/Sidebar.svelte`, `components/TitleBar.svelte`, `components/StatusBar.svelte` | Context `orch`, 4 tabs, one pane (project pane), dock Agents / All projects with 3 agents. 1512 shows the sidebar tree with carets (state remembered) and `+` Add project (tooltip `Add project · ⌘N`); 1280 shows the icon rail. |
| `03-context-select-open.png` | `Workspace.svelte` context Select | Option list: **All projects** (count = all tabs) · each project (count = tabs shown in it; waiting dot when `counts(p.id).needs_you > 0`, `lib/now.svelte.js:counts`). ✓ on the current. |
| `03-context-all-projects.png` | same | Context All projects: every tab shows, Mission control active, Mission control header "every project · follows the project context", **no project row/mark highlighted** in the sidebar / icon rail. |
| `03-empty-workspace-project.png`, `03-empty-workspace-all-projects.png` | `Workspace.svelte` empty state | No tab for this context: one muted line "⌘K to open anything" + two buttons: project context → **Open <project>** (`openProject`) and **New session** (`app.overlay = {kind:'new-session'}`); All projects → **Mission control** (`openMission`) and **New session**. |
| `03-tab-overflow-1280.png`, `03-tab-overflow-states.png` | `TabStrip.svelte` | 12 tabs in a 890 px centre: at start (`‹` disabled), scrolled (both enabled, fades on both edges), at end (`›` disabled; active tab scrolled into view). |
| `03-dock-agents-filtered.png` | `Dock.svelte` | Agents view filtered to a project, waiting dot on the view Select (an agent outside the current filter waits), tooltip on the dot "<agent> (<project>) is waiting for you". |
| `03-dock-shells.png` | `Dock.svelte`, `TermPaneView.svelte`, `lib/app.svelte.js:title` | Shells view, 2 shells with live titles `zsh · ~/dev/orch/ui/src/components`; tooltip = full path. |
| `03-title-status-banner.png` | `components/TitleBar.svelte`, `PacingIndicator.svelte`, `StatusBar.svelte`, `Banner.svelte` | Title bar (no pills; tooltips with keys; pacing popover with **Week planner** button); status bar information only (+ PR build / update chips only when present, `LEADER` only while the leader is held); the 7 banner kinds with their final buttons. |

### 1.3 `04-*` Screens

| PNG | Anchors | States |
|---|---|---|
| `04-agent-strips.png` | `panes/AgentPane.svelte` (header :124-141, strips :148-194) | Live/not-live × permission, needs you; queued; blocked by limit (Resume disabled, tooltip "Blocked until the reset · hh:mm"); error; suspended; watch bar (§1.3.1). |
| `04-agent-waiting.png`, `04-agent-suspended.png` | same, in the app | The app banner never carries the agent condition; only the strip does. `04-agent-suspended.png` shows the header `⋯` Menu open. |
| `04-agent-inspector.png` | `components/inspector/AgentInspector.svelte` | Details + `Open session` + `⋯` Menu (same items as the pane header). |
| `04-feature-pane.png` | `panes/FeaturePane.svelte` | Header Publish/Refresh in Notion · Board · Canvas · Pause/Resume team · Stop · Delete…; Tasks card has no Board button; message box: To Select + input + Send icon button. |
| `04-board-pane.png` | `panes/BoardPane.svelte` | Card menu = Menu (keys right-aligned): Open feature, Run off-hours, Edit, Open remote, Move to, Assign; no footer legend. |
| `04-mission-control-context.png` | `panes/MissionPane.svelte` | Context `billing`: header "in billing · follows the project context", empty "Nothing in billing · Show all projects" (sets the context to All). |
| `04-settings-integrations-1280/1024.png` | `components/SettingsWindow.svelte` (new, §3) hosting `IntegrationsSettings.svelte` | Window 960 wide (944 at 1024), nav without Keyboard. Linear row editing: input + Save + Cancel. |
| `04-project-settings-files-1280/1024.png` | `ProjectSettingsWindow.svelte` (new) hosting `FilesSettings.svelte` | Title "Project settings · orch"; Files with the `read` / `read & write` Select per row. |
| `04-week-planner-1280/1024.png` | `PlannerWindow.svelte` (new) hosting the old `PlannerPane` body | Window 1100 (app − 80 at 1024). Buttons Shadow mode, Sprint mode, Pause all, Weekly report, Edit plan…; Defer / Keep running; Governor log with Undo. |
| `04-keyboard-shortcuts-1280/1024.png` | `ShortcutsSheet.svelte` (new) | Two-column groups of audit §7, search in the title bar, real bindings only. |
| `04-mission-control-context.png`, `04-board-pane.png`, `04-feature-pane.png` | (above) | Also the reference for "project-bound vs app-wide" (§5.3). |

#### 1.3.1 Agent pane rules (what each part does per state)

- **Header** (`AgentPane.svelte`): role mark · id · `StatusBadge` (the only place the state label shows) · model · ctx bar · session pp · `WaitingChip` (only when no strip shows) · `UndeliveredChip`. Buttons: **Interrupt** and **Suspend** only while live; then the `⋯` Menu in every state. No `Resume`, no `⏎`/`esc`/`⌘K` hints (lines :136-141 go).
- **`⋯` Menu** (shared `components/agent/AgentMenu.svelte`, used by header and `AgentInspector`): Shell in its worktree · Diff against base · Open in kitty | Run off-hours (checkable: toggles in working hours) | Restart fresh… · **Kill…** · **Delete…** (destructive, confirm dialog as today). Items disabled by state show a reason tooltip.
- **Strip** (single home of Resume): permission → text + `also in Decisions` link (`focusCard(id)`) + **Allow once** (primary), **Deny**, **Always allow**, **Answer in the pane** (live) or **Resume** (not live: the session that asked is gone). needs you → text + WaitingChip; not live adds **Resume**. queued → **Run now ≈ +x pp week**, **Cancel**. blocked by limit → text + **Resume** disabled. error → **Resume**. suspended → **Resume**.
- **Watch bar**: "Watching <id> · your keys go to Mission control" + **Take over** (`takeOver`, tooltip `Take over · ⏎`) + **Back to Mission control** (`leaveWatch`, tooltip `Back · esc`). `⏎`/`esc` still work (key handlers exist, only their hints go).
- The `⌘⇧I` text in the strips (:148, :162) goes; the "also in Decisions" link replaces it.

---

## 2. New components

### 2.1 `Select.svelte`

```
<Select
  value={…} onchange={(v) => …}          // single value; `bind:value` also works
  options={[{ value, label, group?, count?, dot?, disabled?, reason?, icon? }]}
  placeholder="Choose a project"          // shown when value is null/unknown
  compact={false}                         // 22 px strip variant (dock, tab strip); default 28 px like input.field
  searchable="auto"                       // 'auto' = options.length > 8; true/false forces
  error={null}                            // string: red border + caption under the box
  disabled={false} id name aria-label aria-labelledby
  width="fill" | "hug"                    // compact hugs the value, max 220 px then ellipsis
/>
```

Look: PNG `02-select.png`; tokens in its caption (`input.field`, `border.strong`, `radius.sm`, focus ring 2 px `color.focus` offset 1; open border `color.accent.default`, chevron up; disabled `border.subtle`/`text.disabled`; error `color.status.error` + `text.caption` message). Popover: `radius.md`, `surface.pane`, `border.strong`, `shadow.overlay`, min width = trigger (≥ 200), max 360, max height 320 then scroll; opens below, flips above when there is no room (the dock is at the bottom: `03-dock-*` open upwards). Group headers `text.caption muted`; selected row ✓ at the left; count right-aligned `text.caption muted`; dot = `color.status.waiting-for-you` 7 px; disabled option `text.disabled`, its `reason` as a tooltip.

Keyboard (listbox pattern): trigger **↓ / ↑ / Enter / Space** opens; in the list **↑ ↓** move, **Home / End**, **type-ahead** (500 ms buffer; with search the typed text goes to the filter instead), **Enter / Space** picks and closes, **Esc** closes and keeps the value, **Tab** closes (keeps value, moves on). Disabled options are skipped. Focus returns to the trigger on close.

ARIA: trigger `role="combobox" aria-haspopup="listbox" aria-expanded aria-controls aria-activedescendant`; list `role="listbox"`, group `role="group" aria-label`, option `role="option" aria-selected aria-disabled`. With search the input is `role="searchbox"` inside the popover and the trigger keeps `combobox`. Error: `aria-invalid` + `aria-describedby` the caption. Render the popover in a portal on `document.body` (so a clipped container, the dock or a dialog, never cuts it); close on outside `pointerdown`, on scroll of an ancestor and on window blur.

Form integration: Select renders a hidden `<input name value>` so form-based code keeps working; `onchange` fires only on a user pick. In `Dialogs.svelte` the `⌘⏎` submit and `Esc` cancel handlers (`:226`, `:409`) must ignore an Esc that closed an open Select popover (check `e.defaultPrevented`, Select calls `preventDefault` on the Esc it consumes).

**Shared lists** (new `lib/options.js`): `MODELS` (default + 4 aliases + custom, with fable) used by `ModelSelect`, `Dialogs` new session, `OverwatchSettings`, `RoleMcp`, `TeamCanvasPane`; `EFFORTS`, `PERMISSION_MODES`, `TIERS`, `REVIEW_MODES`, `PERMISSION_PROFILES`, `BUDGET_PRIORITIES`, `BOARD_COLUMNS` (the static lists of §2.1.1). Name clash: `lib/ecosystem.js` already exports `MODELS = ['opus','sonnet','haiku']` (RoleMcp, no fable) and `lib/canvas.js` has `MODEL_ALIASES` (4, with fable): replace both with the new list, do not add a third. The **required-value** pickers (Overwatch model, RoleMcp downgrade target) use the aliases only: no "default" and no "custom…" entry (`MODEL_OPTIONS` = aliases, `MODELS` = default + aliases + custom for the optional ones); `RoleMcp.defaultTo` and `downgradeProblem` (`family()`) must be checked with the 4th alias, and the core must accept it for the Overwatch model before it is offered there. Disabled model options carry `reason` ("Not in your plan (Max only)", `02-select.png`) when the plan lacks them. `ModelSelect.svelte` becomes a thin wrapper around Select + the "custom…" text input.

#### 2.1.1 The 27 native `<select>` (all to Select)

Lines from the audit base; use `grep -rn "<select" ui/src` to relocate. After the change `grep -rn "<select" ui/src` returns nothing (test, §10).

| # | file:line | what | options |
|---|---|---|---|
| 1 | `components/AllocationSettings.svelte:195` | allocation tier | `TIERS` (4) |
| 2 | `components/McpSettings.svelte:306` | design tool of a server | 4 static |
| 3 | `components/RoleMcp.svelte:63` | downgrade target model | `MODELS` (3 → shared list) |
| 4 | `components/RoleMcp.svelte:67` | downgrade trigger | 3 static |
| 5 | `components/PermissionsSettings.svelte:134` (one element, rendered for team and solo) | default effect | 2 static |
| 6 | `components/ModelSelect.svelte:24` | model (used twice by `TeamCanvasPane:647-648`) | `MODELS` + custom… |
| 7 | `components/OverwatchSettings.svelte:151` | Overwatch model | `MODELS` |
| 8 | `components/PlanEditor.svelte:85` (per row) | tier | `TIERS` |
| 9 | `components/Dialogs.svelte:239` | new session: project | dynamic; search when > 8 |
| 10 | `components/Dialogs.svelte:244` | new session: role | dynamic |
| 11 | `components/Dialogs.svelte:259` | new session: model | `MODELS` (default + models) |
| 12 | `components/Dialogs.svelte:264` | new session: effort | `EFFORTS` (4) |
| 13 | `components/Dialogs.svelte:269` | new session: permission mode | `PERMISSION_MODES` (3) |
| 14 | `components/Dialogs.svelte:294` | launch: project | dynamic |
| 15 | `components/Dialogs.svelte:299` | launch: team | dynamic; invalid teams `disabled` + `reason` |
| 16 | `panes/ProjectSettingsPane.svelte:243` | Linear team | dynamic, searchable |
| 17 | `panes/ProjectSettingsPane.svelte:249` | Linear project | dynamic, searchable |
| 18 | `panes/ProjectSettingsPane.svelte:265` (per row) | board column for a Linear state | `BOARD_COLUMNS` (5) |
| 19 | `panes/BoardPane.svelte:239` | new task: feature | dynamic |
| 20 | `panes/BoardPane.svelte:244` | new task: node | dynamic |
| 21 | `panes/BoardPane.svelte:265` | assign task to node | dynamic |
| 22 | `panes/FeaturePane.svelte:337` | message recipient (compact) | dynamic |
| 23 | `panes/TeamCanvasPane.svelte:642` | node role | dynamic |
| 24 | `panes/TeamCanvasPane.svelte:651` | role effort | `EFFORTS` |
| 25 | `panes/TeamCanvasPane.svelte:662` | permission profile | `PERMISSION_PROFILES` (3) |
| 26 | `panes/TeamCanvasPane.svelte:669` | budget priority | 4 static |
| 27 | `panes/TeamCanvasPane.svelte:699` | review mode | `REVIEW_MODES` (3) |

(27 `<select>` elements, checked with `grep -rn "<select" ui/src` at the audit base. Not a `<select>` today but drawn as a Select in `04-project-settings-files-*`: the per-file access control of `components/FilesSettings.svelte:319` (a radio pair `read` / `write`): replace it with the same Select, 2 static options `read` / `read & write`; keep `setAccess`.)

Also move to the new components: the ad-hoc popovers listed in §2.2 and `components/mission/ProjectFilter.svelte` (deleted, §5.3).

### 2.2 `Menu.svelte`

```
<Menu items={[{ id?, label, icon?, key?, destructive?, disabled?, reason?, checked?, items? /* submenu */, separator? }]}
      onselect={(item) => …}              // or each item has run()
      trigger="icon" | snippet            // default: the `⋯` icon button (aria-label from `label`)
      label="More actions" />
```

Same popover as Select (§2.1): item height 28, icon `text.icon`, label `text.body`, right-aligned `Kbd` (`key`), separator 1 px `border.subtle` + 4 px above/below, destructive = `color.status.error` label + icon with an error-tint highlight, disabled = `text.disabled` + `reason` tooltip, items that open a dialog end with `…`. Right edges aligned with the trigger. Trigger tooltip is the label only ("More actions"), no key (the menu has none of its own).

Keyboard: trigger Enter / Space / ↓ opens (first item highlighted); ↑ ↓ Home End type-ahead; **Enter** runs and closes; **→** opens a submenu, **←** closes it; **Esc** closes, focus back to the trigger. ARIA: trigger `aria-haspopup="menu" aria-expanded`; list `role="menu"`; items `role="menuitem"` (`menuitemcheckbox` + `aria-checked` for checkable); separators `role="separator"`; destructive is visual only. Keys shown in items are shown **only where the key works in that place** (the agent pane menu has none; the Board card menu has its row keys; the palette entry for `⋯` is not needed).

Where Menu is used (each replaces an existing ad-hoc control): AgentPane header `⋯` (replaces "More ⌘K" :141) and AgentInspector (§1.3.1); BoardPane card menu (`BoardPane.svelte:296-301`, keys kept right-aligned); TeamCanvasPane connect/add menus (`:576-614`, keys right-aligned); `Menu` in the sidebar rows is **not** added (out of scope).

### 2.3 `TabStrip.svelte` (workspace) and the dock strip

Props: `tabs=[{id, title, icon, badge?, closable}]`, `active`, `onselect`, `onclose`, `compact`, snippet `leading` (the Select(s)), snippet `trailing` (dock: `+`, Hide dock).

- Tab: kind icon (`text.icon` 12) · full title (`text.label`) · × (hover/active/focused only). `min-width: content`, `max-width: 280px` then ellipsis (title in the tooltip when cut). **Never shrinks**: `flex: none`. Active: `surface.pane`, 2 px `color.accent.default` top bar; hover `surface.raised`; keyboard focus ring inside the tab. Middle-click closes. Order is insertion order (no drag reorder in this feature).
- Overflow: the lane scrolls horizontally (wheel/trackpad), 24 px fade on a clipped edge; **`‹ ›`** icon buttons (22 px) at the right end of the strip, shown only when `scrollWidth > clientWidth` (a `ResizeObserver`, like `Dock.svelte:measure`), each `disabled` at its end (tooltips "Scroll tabs left / right"). A click scrolls by `clientWidth − one tab`; press-and-hold repeats (400 ms, then 80 ms). Active tab and any tab just opened are scrolled into view (`scrollIntoView({inline:'nearest'})` after `tick()`), as `Dock.svelte` does today.
- ARIA: strip `role="tablist"` `aria-orientation="horizontal"`; tab `role="tab" aria-selected` + roving `tabindex` (← → move, Home/End, Enter activates, Delete/⌘W closes); the pane is `role="tabpanel" aria-labelledby`. The two arrows are plain buttons with `aria-label`, outside the tablist order.
- Workspace strip leading snippet = the context Select (§5.1); dock strip leading = view Select + project Select (§5.2).

### 2.4 `FloatingWindow.svelte`

```
<FloatingWindow title="Settings" subtitle="orch" size={{ w: 960, h: 760 }} onclose={…}
                nav={[{id,label,badge?}]} section onsection={…}>      // nav optional
  {@render children()}
  {#snippet titleExtra()}…{/snippet}                                   // Keyboard sheet: the search field
</FloatingWindow>
```

- Rendered by `App.svelte` as one `{#if app.window}` block above the status bar, **outside** the workspace, sibling of `Dialogs`. Backdrop: black at `opacity.backdrop` 0.6 over the whole app (title bar included); a click on it closes. Window: centred, `radius.lg`, `surface.pane`, `border.default`, `shadow.overlay`; title bar 36 px (`surface.chrome`, `text.title`, project name after "· " in `text.secondary`), close × icon button (tooltip `Close · esc`); optional nav 200 px (`surface.chrome`; item 28 px, selected `surface.selected` + 2 px `color.accent.default` bar), content padding `space.xl`, scrolls inside.
- Sizes: Settings 960 × min(760, vh − 80); Project settings the same; Week planner 1100 × same; Keyboard shortcuts 720 × same. Width = min(size.w, vw − 80), never under 720 (so at a 1024 window: 944). Height follows the same cap. The layout below 840 wide stacks the planner's two-column cards (`PlannerPane`'s `w` measure already does this, keep it).
- Accessibility: `role="dialog" aria-modal="true" aria-labelledby`. **Focus trap** (Tab/Shift+Tab cycle inside; use the `inert` attribute on `#app-root`'s other children while open, with a Tab-wrap fallback), initial focus = the first nav item (or the first field for no-nav windows, or the search field in the sheet), **restore focus** to the element focused before opening (store `document.activeElement`; fall back to the active tab). Esc closes (but first lets an open Select/Menu popover or an editing field consume it: `if (!e.defaultPrevented)`). The nav is a `role="tablist" aria-orientation="vertical"` + `tabpanel`; `j`/`k` and ↑/↓ move the section, Enter moves focus into the content (today's `SettingsPane.onNavKey`/`onBodyKey` logic, kept), Esc in the content goes back to the nav first, then closes.
- Global keys while a window is open: ⌘-chords are handled like with a dialog (`App.svelte:onKeyDown`, "A dialog keeps the chords to itself") **except** ⌘K / ⌘P (the palette opens above the window and the window stays open), ⌘T / ⌘N (their dialog opens above the window, §3.3), ⌘W (closes the window) and ⌘, / ⌘/ (⌘, on the open Settings closes it, on another window replaces it; see §3.3). The `inert` / focus trap covers the app root only: the overlay layer (`Palette`, `Dialogs`) is rendered after the window and is not made inert, so it stays typeable above the window.
- Only one window at a time: `app.window = { kind, section?, server?, projectId? } | null`; opening another replaces it (no stack).

### 2.5 `Tooltip` (`lib/tooltip.js`, action `use:tooltip={{ label, key?, detail? }}`)

Look and rules: `02-tooltip.png`. 500 ms after hover, at once on keyboard focus, hides on leave / blur / Esc / click; never holds a control; `Label · key` with the key as a `Kbd` after a muted `·`; two-line variant (title + `text.caption muted` detail) for a full value (shell path, disabled reason). Below the control, 6 px gap, flipped above at the bottom of the window, kept inside the window. Replaces native `title=` on every control the audit touches (native titles stay on non-interactive truncated text). `aria-describedby` points to the tooltip while shown; icon-only buttons always also get `aria-label`.

---

## 3. Floating windows: how things open

### 3.1 State and opener functions (`lib/app.svelte.js`)

Add `app.window = null` (not persisted: a window never reopens at launch, and `restoreLayout` drops any saved pane of type `settings` | `project-settings` | `planner` without opening it).

| Function (today) | After |
|---|---|
| `openSettings(section, server, projectId)` (:421) | `app.window = { kind: 'settings', section: section ?? 'integrations', server, projectId }`. If a Settings window is already open: update `section`/`server` in place (focus stays in the window, nav follows). If another window is open: replace. No `addPane`, no `findPane`. `projectId` is accepted only so callers can be rewritten: **a non-null `projectId` with section `mcp` is rewritten to `openProjectSettings(projectId, 'mcp')`; with any other section it is ignored** (`fixSource` in `intake.svelte.js` calls `openSettings('integrations', 'github' | 'linear', projectId)`: the project only says where the error came from, the Integrations window opens as for a null `projectId`) (audit §2.5): palette `settings.mcp` project variant (:1264), `RoleMcp` (:35/:48), `Dialogs` (:323), `routeOpen` (:466). |
| `openProjectSettings(projectId, section)` (:431) | `app.window = { kind: 'project-settings', projectId, section: section ?? 'backend' }` (default section stays `backend` until General is designed; the nav lists General first as a placeholder). `toast('No project in focus')` guard stays. One window total, not one per project: opening it for another project replaces it. |
| `openSingleton('planner')` (:379) | `openPlanner()`: `app.window = { kind: 'planner' }`. `openSingleton` keeps handling `mission`, `setup` only. Palette `planner`, leader `g`, banners, PacingIndicator popover, Settings › Pacing link, ReportPane Esc call `openPlanner`. |
| (new) `openShortcuts()` | `app.window = { kind: 'shortcuts' }`; palette `shortcuts` ("Keyboard shortcuts"), chord `cmd+/` in `lib/keyboard.js:CHORDS`, Help menu item (§3.4). |
| (new) `closeWindow()` | `app.window = null`; restores focus (below). |

`App.svelte` mounts `<SettingsWindow>`, `<ProjectSettingsWindow>`, `<PlannerWindow>`, `<ShortcutsSheet>` from `app.window.kind`. The three section bodies are the **current components, unchanged in logic**: `SettingsPane.svelte` → `components/SettingsWindow.svelte` (same `SECTIONS` without `keyboard`; `pick()` writes `app.window.section` instead of `app.panes[paneId]`; drop `saveLayout()` calls), `ProjectSettingsPane.svelte` → `ProjectSettingsWindow.svelte` (`:171` the same), `PlannerPane.svelte` → `PlannerWindow.svelte`. Remove their `title()` cases and the `'settings' | 'project-settings'` entries of `PaneFrame`'s `kind` map and of `PANE_KEYS` users (`reopenClosed`: a closed settings pane is not reopened; closing a window does not push to `closed`).

### 3.2 Deep links (what opens what)

| Source | Call | Window result |
|---|---|---|
| ⌘, / palette `settings` | `openSettings()` | Settings on Integrations (or its last section: keep `app.settingsSection` in memory, not persisted) |
| palette `settings.integrations`, `settings.mcp`, `overwatch.settings`, `update.pr` | `openSettings(id)` | that section |
| toast "Restart to run" (:1149), StatusBar PR chip, ChatInput "turn on" link, `IntegrationsSettings` links | `openSettings('builds', 'pr:<n>')` etc. | section `builds` / `overwatch` with the server/PR preselected, as today |
| decision card `routeOpen({open:'settings', server}, projectId)` | `openSettings('mcp', server)` or, with a project, `openProjectSettings(projectId,'mcp')` | |
| ProjectPane Settings button, palette `project.settings.<id>` | `openProjectSettings(id)` | Project settings · <name> |
| ProjectSettingsPane links to Settings (:237, :311) | `openSettings('integrations')` | the Settings window **replaces** the Project settings one |
| banner `week` / `5h` / `red` / `ac` button, PacingIndicator popover button, Settings › Pacing intro link, palette `planner`, leader `g`, ReportPane Esc | `openPlanner()` | Week planner |
| Allocation › **Edit weekly plan…** | opens the plan dialog (`Dialogs` `plan` overlay) over the window | label fixed; the dialog is above the window (z-order, §3.3) |
| ⌘/ , Help › Keyboard shortcuts, palette `shortcuts` | `openShortcuts()` | Shortcuts sheet |

### 3.3 Focus, keys and stacking

- **Open**: remember `document.activeElement` (`app.windowReturn`, an element reference, not persisted; if it is gone at close, focus the active tab via `focusPane(app.focused)`), then focus per §2.4. Replacing a window does **not** overwrite `windowReturn` (the original opener stays).
- **Close** (Esc, ×, backdrop click, `closeWindow()`): `app.window = null`, restore focus; a terminal pane regains the keyboard through `focusPane`'s existing microtask.
- **⌘,** with Settings open: close it (toggle). With another window open: replace by Settings. **⌘/** likewise for the sheet. Other global chords (⌘K palette, ⌘P jump) open the palette above the window (`Palette` z-index > window); ⌘T / ⌘N open their dialog above the window; the dialog layer is above the window layer. ⌘W with a window open closes the window, not a tab.
- Dialogs (`Dialogs.svelte`: new session, add project, launch, plan, delete, confirm) stay `app.overlay` dialogs, drawn with the **same shell** (backdrop, title bar, ×, focus trap) at their current small sizes: factor `Dialogs.svelte`'s `.dlg` wrapper into `FloatingWindow` (no nav, `size` per dialog). The Cancel/submit buttons lose their `esc` / `⌘⏎` / `⏎` pills; tooltips "Cancel · esc", "Start session · ⌘⏎".
- Settings saves per field as today; no confirm on close; the Week planner and Settings keep polling/`onTopics` while open and stop on close (`onMount` cleanup already does it).

### 3.4 Native menu and Help

`crates/orch-app/src/menu.rs:ITEMS`: remove `pane.focus.1…9` ("Focus Pane n") and re-add as "Tab 1…9" (`pane.focus.n` ids stay: they are tab indexes now, §4.3); add `("shortcuts", "Keyboard Shortcuts", "CmdOrCtrl+/")` under a new **Help** submenu (the Tauri `SubmenuBuilder` "Help"); the accelerator `CmdOrCtrl+/` must be in `CHORDS` (`'cmd+/': 'shortcuts'`) because the webview handles chords first. `lib/keyboard.js:chordName`: `/` is a shifted key on AZERTY layouts (e.key is `'/'` with `shiftKey`, so the name would be `cmd+shift+/`): like `Semicolon`, normalise it there (when `e.key === '/'` drop the `shift` modifier) and add a `keyboard.test.js` case for both layouts. Close Pane label → "Close Tab".

---

## 4. Single pane

### 4.1 Commands and keys removed

| Command id | Key | Remove from | Notes |
|---|---|---|---|
| `pane.split.right` | leader `v` | `lib/keyboard.js:LEADER`, palette list in `commands()` (`lib/app.svelte.js`), `run` map, `menu.rs` if present | `openShell({how})` loses `how`; a shell always goes to the dock |
| `pane.split.below` | leader `s` | same (:1284) | |
| `pane.zoom` | leader `z` | `LEADER`, :1285, `toggleZoom` (:774), `app.zoomed`, `dockFills` zoom branch (:106), `Dock.svelte` ZOOMED tag + `restore` button, `Workspace.svelte`'s zoomed `PaneFrame` branch | `dockFills()` becomes "dock open and `app.layout` is null" (no tab at all); it must **not** key on the context, or a context with no visible tab would hide the workspace strip with the context Select and the empty-state buttons, leaving no way back from the strip |
| `focus.left`, `focus.right` | leader `h` / `l` | `LEADER`, `run` map (:1396-1399), `L.neighbor` | |
| `focus.up`, `focus.down` | leader `k` / `j` | **kept**: `focusDirection` becomes: down = into the dock (`app.dock.active`), up = out of it to the workspace's active tab. `L.neighbor` is no longer used. | |
| `pane.focus.1…9` | ⌘1…9 | **kept, re-meant**: tab n of the *visible* tabs of the current context (`focusNumber`, :768). Palette label "Tab n". `menu.rs` labels. | Out of range: nothing (no toast). |
| `pane.close` | ⌘W | **kept**: closes the focused tab (workspace tab or dock tab); label "Close tab". With a window open it closes the window (§3.3). | |
| `pane.reopen` | ⌘⇧T | kept, label "Reopen closed tab" | |
| (new) `shortcuts` | ⌘/ | add | §3 |
| `dock.next` / `dock.prev` | leader `]` / `[` | kept: cycle **within the current view and filter** (`cycleDock`, :263) | |

Update `lib/keyboard.js:hint()` (it feeds palette rows) and the palette ids in `app.js:commands()` (:1283-1285 removed). `test/keyboard.test.js` asserts the table (§10). Anything else calling `openShell({how:'right'|'below'})` or `addPane(..., 'right'|'below')` passes a tab/dock instead: `openEditor` (nvim) → a workspace **tab** (`addPane(spec, 'tab')`, project-bound, §5.3); `revive`/`reopenClosed` use `'tab'` (`isTerminal(s) || !app.layout ? 'right' : 'tab'` → `'tab'`).

### 4.2 Layout model and migration

Keep `app.layout` as the **single leaf** `{ kind:'leaf', id, tabs: paneId[], active }` or `null` (so `L.leaf/addTab/activate/remove/visible` stay as they are); delete `L.split`, `L.neighbor`, `L.rects`, `L.resize`, and `Node.svelte` (its drag/nudge handlers). `addPane(spec, how)` loses `how`: always `L.addTab` (or `L.leaf` when empty); `app.zoomed` and every read of it go.

Migration (`restoreLayout`, `lib/app.svelte.js`, and the new `L.flatten`):

```js
// lib/layout.js
export function flatten(tree, focused) {            // a saved tree (leaf or splits) → one leaf, or null
  const ids = allPanes(tree);                       // children order = reading order (left→right, top→bottom)
  if (!ids.length) return null;
  const active = ids.includes(focused) ? focused : visible(tree)[0];
  return { kind: 'leaf', id: nid('l'), tabs: ids, active };
}
```

`restoreLayout` calls `app.layout = L.reid(L.flatten(L.prune(saved.layout, exists…), saved.focused))`. Saved `panes` of type `settings` / `project-settings` / `planner` are dropped before (their ids removed from `ids`). `app.zoomed` was never saved: nothing to migrate. The saved `layout` key stays the same shape (a leaf is already a valid old tree), so a downgrade still loads. Missing `workspace` / new `dock` keys use the defaults of §5.4.

### 4.3 What renders

`Workspace.svelte` (38 lines today) becomes: `<main class="ws"> <TabStrip …leading={context Select}> {#if activeTab}<PaneHost paneId={activeTab} />{:else}<empty state />{/if} </main>`. `PaneHost` is `PaneFrame.svelte` minus its header (number, kind, zoom hint, close hint: lines :31-39 go; `class:focused` accent border stays only if the design's pane has it: it does not, the active tab is the cue). The body switch by `spec.type` is unchanged except `settings | project-settings | planner` are gone. Hidden (non-active or filtered-out) panes do not render; their `TermPane`s (editors, shells) keep running and keep their buffers (`terms` map is untouched, `focusPane`'s microtask still focuses the xterm of the active tab).

Pane header content that PaneFrame showed (kind label + title) is now the tab; the pane's own header (ProjectPane, FeaturePane…) is unchanged.

---

## 5. Project context, dock views and filter, persistence

### 5.1 Workspace project context

State: `app.workspace = { project: 'all' | projectId }` (api-contract `workspace.project`). Helpers in `lib/app.svelte.js`:

```js
export const paneProject = (spec) => ...   // project id a pane belongs to, or null = app-wide (§5.3)
export const visibleTabs = () => (app.layout?.tabs ?? []).filter(inContext)
const inContext = (id) => app.workspace.project === 'all' || paneProject(app.panes[id]) == null || paneProject(app.panes[id]) === app.workspace.project
export function setContext(projectId /* 'all' | id */)   // sets, activates a visible tab, saveLayout()
```

Rules:
1. **Filter**: tabs not in the context are hidden from the strip but stay open (not closed, terminals keep running); changing the context shows them again.
2. **Active tab on change**: keep the active tab if it is still visible; else the last-active visible tab for that context (remember `app.workspace.last[projectId]` in memory), else the first visible; else the empty state (§1.2 `03-empty-workspace-*`).
3. **Opening a pane of another project** (`focusPane` on a hidden tab, `openProject`, `openFeature`, `openBoard`, `openDiff`, `openCanvas` with a project): `setContext(thatProject)` first — except when the context is `all` (stays `all`). App-wide panes never change the context.
4. **Sidebar / icon rail**: clicking a project sets the context (`openProject` already sets `app.selectedProject`; it now also calls `setContext`). Clicking a feature or agent row there: feature → its project's context + `openFeature`; agent → `openAgent` (dock; the context is **not** changed). In `all`, no project row/mark is highlighted: `Sidebar.svelte` and `IconRail.svelte` derive "current" from `app.workspace.project !== 'all' ? it : null` (today `app.selectedProject`; keep `selectedProject` for the inspector but not for the highlight).
5. **Restore**: unknown project id → `'all'` (api-contract).
6. **Default** when no saved value: the focused project, else `all`.
7. Context Select (`Workspace.svelte`, compact): options `All projects` + `projects()`; counts = visible tabs per project; waiting dot via `counts(p.id).needs_you > 0` (`lib/now.svelte.js:counts`); the closed value shows the project name; width hugs (max 220). Palette `mission.filter` (:1220) becomes "Set the project context…" → opens this Select (`app.contextOpen = true`); `missionFilterOpen`, `app.mission`, `setMissionFilter` and the `mission` settings key are removed (the key is ignored on read).

### 5.2 Dock views and filter

State in `app.dock` (extends `{open, height, tabs, active, v}`; keep `v: 2`): `view: 'agents'|'shells'`, `agentProject: 'all'|projectId`, `activeAgent`, `activeShell` (pane ids), with `active` = the active tab of the current view. Derived in `app.js`:

```js
export const dockAgents = () => app.dock.tabs.filter((id) => app.panes[id]?.type === 'agent')
export const dockShells = () => app.dock.tabs.filter((id) => app.panes[id]?.type === 'shell')
export const dockTabs  = () => app.dock.view === 'agents'
  ? dockAgents().filter((id) => app.dock.agentProject === 'all' || app.panes[id].projectId === app.dock.agentProject)
  : dockShells()
```

- **View Select** options `Agents` / `Shells` with counts `n` = the number of **open dock tabs** of that kind, all projects, independent of the project filter (audit §8; "not every agent of the sidebar"). A **waiting dot** shows on the Select when an agent tab that is *not* shown (the other view, or another project under the filter) has `attention(agentId)` (`lib/now.svelte.js:attention`); the tooltip names it ("lead (orch) is waiting for you").
- **Project Select** (Agents view only): `All projects` + projects that have at least one **open agent tab**, count = their tabs, waiting dot per project. A saved/active `agentProject` with no open agent tab left is still listed (count 0) while it is the filter, so the Select never shows an unknown value; closing its last tab keeps the filter and shows "No agent running in <project>".
- **Opening** (`addDockPane`, `openAgent`, `openSession`, `focusPane` on a dock tab): an agent → `view='agents'`; filter stays `all`, or becomes the agent's project **only if** the filter is a different specific project and the agent's tab would be hidden (so the tab you asked for is visible); a shell → `view='shells'`. `focusPane` sets `dock.active` and `activeAgent`/`activeShell` for its kind.
- **Switching view** restores that view's `activeAgent` / `activeShell` (else the first tab of the view) and focuses it if the dock has the focus.
- **Empty view**: view `shells` with no shell → "No shell open" + inline **New shell** button (no `+` in the strip then: one home); view `agents` filtered to a project with none → "No agent running in <project>". Closing the last tab of a view switches to the other view when it has tabs, else closes the dock (as today: `closePane`, :302-304). `toggleDock` with no tabs at all still opens a new shell.
- **Strip right end**: `‹ ›` when overflowing, then `+` (**New shell**, Shells view only; `openShell()`), then the **Hide dock** icon button (tooltip `Hide dock · ⌘J`; replaces the `⌘J hide` hint :128). The `ZOOMED` tag and `⌘; z restore` are gone.
- **Tab** content: agent = role mark (`AgentChip compact`) · short name · `StatusBadge compact` (status icons use the StatusBadge vocabulary) · ×; shell = title (§6) · ×; command shells show their fixed title. Tooltips as `Dock.svelte:tip` today, plus the full path for shells.

### 5.3 Project-bound vs app-wide panes

`paneProject(spec)` returns a project id for these and `null` (app-wide, always visible) for the rest:

| Pane type | Project-bound? | `paneProject` |
|---|---|---|
| `project` | yes | `spec.projectId` |
| `feature` | yes | the feature's `project_id` (`features().find(f => f.id === spec.featureId)`) |
| `board` | yes | `spec.projectId` |
| `diff` | yes | `spec.projectId` |
| `canvas` | yes when opened for a project's feature/team | `spec.projectId ?? team's project`; a global team canvas: app-wide |
| `editor` (nvim) | yes when it has a project | `spec.projectId ?? null` |
| `trust` | yes | `spec.projectId` |
| `mission` | **no** (app-wide) — but its rows follow the context | `null` |
| `setup` | **no** | `null` |
| `report` (Weekly report) | **no** (app-wide: the report is the whole plan week) | `null` |
| agent, shell | dock only; not in the workspace strip (the dock has its own filter) | — |
| `settings`, `project-settings`, `planner` | windows, not panes | — |

**Mission control** reads `app.workspace.project` instead of `app.mission.projects`: `MissionPane.svelte` (`:54` filter derivation, `:78`, `:200` `f` key, `:230-239`, `:294` `ProjectFilter`, the Filter button and toolbar hint) lose all filter UI; rows use `lanes(...)` filtered by `project === ctx` (`lib/mission.js:lanes` takes a `projects: string[] | null` argument today: pass `[ctx]` or `null`). Header: "Mission control" + muted "in <project> · follows the project context" or "every project · follows the project context". Empty: "Nothing in <project>" + **Show all projects** (`setContext('all')`). `mission-fixture.js` / `mission-pane.test.js` adapt.

### 5.4 Persistence (saved with `saveLayout`, `lib/app.svelte.js`)

The `layout` setting value gains keys (api-contract): `sidebarTree`, `workspace: { project }`, `dock: { …existing, view, agentProject, activeAgent, activeShell }`.

| Key | Source of truth | Written | Read in `restoreLayout` |
|---|---|---|---|
| `sidebarTree` `{ [projectId \| "projectId/featureSlug"]: boolean }` | replaces `let collapsed = $state({})` in `Sidebar.svelte:14` (keys already used: `collapsed[p.id]` and `collapsed[k]` with `k = \`f:${f.id}\``: the contract's `"projectId/featureSlug"` format is not what the code keys on; keep `p.id` / `f:<featureId>` and prune on those); store **expanded** booleans in `app.sidebarTree`, default expanded when the key is absent | caret click → `app.sidebarTree[k] = !…; saveLayout()` | `app.sidebarTree = saved.sidebarTree ?? {}`; keys of projects/features that no longer exist are pruned on read |
| `workspace.project` | `app.workspace.project` | `setContext` | unknown id → `'all'`; absent → focused project else `'all'` |
| `dock.view` | `app.dock.view` | view Select | absent → `'agents'` if an agent tab exists else `'shells'` |
| `dock.agentProject` | `app.dock.agentProject` | project Select | unknown id → `'all'` |
| `dock.activeAgent`, `dock.activeShell` | `app.dock` | `focusPane` | pane id not among the restored tabs → first tab of that view; `dock.active` = the one of `dock.view` |

`saveLayout` snapshots `app.dock` already (`$state.snapshot(app.dock)`): the new fields ride along; add `sidebarTree` and `workspace` to the `value` object. Unsaved on purpose: `app.window`, `windowReturn`, the in-memory `workspace.last`, tab scroll position, Select popovers.

---

## 6. Shell tab live title

**Display** (`Dock.svelte`, shell tab; `lib/app.svelte.js:title` shell case): `zsh · <cwd>` — `<shell>` = basename of `$SHELL` (the core knows it: `terms.rs:login_shell`; return it in `TermInfo` or default `zsh`), `<cwd>` = home shortened to `~` (`lib/format.js:shortPath` exists: reuse), truncated **from the left** beyond ~32 chars (`…prove-orchestrator-ui-ux/ui-1`), full path in the tooltip (two-line tooltip, §2.5). Applies to dock shells that were opened by `openShell` / `revive`. **Fixed titles** (no live cwd): command shells (`runInPane`, `spec.run` / `spec.title` set: "gh auth login", "pnpm --filter …") and `trust`. Add `spec.live = true` on interactive shells; `title(spec)` returns the live form when `spec.live && spec.cwd`.

**Sources, in this order** (first that gives a value wins; each update calls `app.panes[id].cwd = …; app.panes[id].title = title(spec)` and `saveLayout()` is throttled 400 ms as today):

1. **OSC 7** (`file://host/path`): in `lib/term.js:TermPane` register `this.term.parser.registerOscHandler(7, (data) => { …; return true })` (xterm.js `allowProposedApi` is already on); decode with `new URL(data)` (`decodeURIComponent` on the path; ignore a host that is not this machine's or empty: an `ssh` session's OSC 7 names the remote host and its path means nothing locally). Hook `hooks.onCwd(path)`.
2. **Core-side cwd poll** (the baseline, §6.1).
3. **OSC 0/2 title** (`term.onTitleChange`): only when it *looks like a path* (`^[~/]`) — shells' default titles are `user@host:path` / `zsh` / the running command, which would flap the tab title. Ignored otherwise.
4. The opening path (`spec.cwd`), as today.

### 6.1 Open choice: shell integration or cwd poll — recommendation

zsh and bash send no OSC 7 by default (macOS's Terminal.app gets it from `/etc/zshrc_Apple_Terminal`, keyed on `TERM_PROGRAM=Apple_Terminal`; the app sets `TERM_PROGRAM=Orchestrator`, `terms.rs:base_env`, and starts from `env_clear`).

| | Shell integration snippet injected at spawn | Core-side cwd poll |
|---|---|---|
| zsh/bash/fish/nu | needs one injector per shell (`ZDOTDIR` shim that sources the user's real `.zshrc` then installs a `precmd`; `--rcfile`/`PROMPT_COMMAND` for bash); fish/nu differ | one mechanism, shell-agnostic |
| user dotfiles | a `ZDOTDIR` override breaks setups that rely on `ZDOTDIR` (`~/.config/zsh`) or on `.zshenv`; hook ordering with the user's prompt frameworks (p10k, starship) is fragile | none touched |
| accuracy | instant, exact, also follows `cd` in nested shells (tmux/ssh send their own) | up to the poll interval (1.5 s); sees the shell process's cwd only (not a subshell inside `ssh`) — which is what you want |
| nested / remote | an `ssh` shell reports a remote path (wrong locally) | local only |
| cost | startup shim, support burden | `proc_pidinfo(PROC_PIDVNODEPATHINFO)` per shell every 1–2 s, a few µs each |

**Recommendation: the core-side poll, with OSC 7 honoured when a shell does send it** (so Terminal-style, kitty-style and user-configured shells are instant, everything else is within a second). No injection, no dotfile risk, one code path for every shell. Implementation:

- `crates/orch-app/src/terms.rs`: `Term` already keeps `pid` (the shell, a session leader). Add `pub fn cwd(&self) -> Option<PathBuf>`: macOS `libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, &mut info, size_of)` → `info.pvi_cdir.vip_path` (the `libc` crate is already a dependency, `Cargo.toml: libc.workspace`); `#[cfg(not(target_os="macos"))]` reads `/proc/<pid>/cwd` (readlink). Use the **shell's pid**, not the foreground process group: when `nvim` or `ssh` runs, the title keeps the shell's directory.
- New Tauri command `term_cwd(id) -> Option<String>` next to `term_resize` (`commands.rs`, registered in `lib.rs` after `term_kill`). This is the only backend addition of the feature (api-contract said none was designed; the notes make it).
- UI: `lib/term.js:TermPane` or a small store `lib/shellcwd.js`: while the app window is visible (`document.visibilityState`), every 1500 ms `invoke('term_cwd', { id })` for each **dock shell tab** with `spec.live` (cheap; skip a shell whose OSC 7 was ever seen: `spec.osc7 = true`), update `spec.cwd`/`title` when it changed. Stop on tab close / window hidden; one immediate read on tab focus.
- Tests: §10 `shellcwd.test.js`. Core: a `terms.rs` unit test that spawns `sh -c 'cd /tmp && exec sleep 5'`-style and checks `Term::cwd()` returns `/tmp` (canonicalised: macOS `/private/tmp`).

---

## 7. Hints and the Keyboard shortcuts sheet

Rule (spec §3.1): no `Kbd` inside or next to a control. Allowed: tooltips (`Label · key`), palette rows and input hint (`Palette.svelte`), Menu items (right-aligned), the sheet, one muted empty-state line, the leader overlay (`StatusBar.svelte` `LEADER`, only while `app.leader`), text-field placeholders (`Message the Overwatch · ⏎ to send`, `Reply · ⏎ to send`).

**Mechanical change**: every `<Kbd keys=…/>` found by `grep -rn "<Kbd" ui/src` is either deleted (key moved to the control's `use:tooltip`) or kept in one of the allowed places. The full per-file list with line numbers is [audit.md §4](audit.md); it is the work list (≈ 40 files). Examples: `TitleBar.svelte:18/:24` → tooltips "Jump or run a command · ⌘K" / "Decisions · ⌘⇧I"; `Banner.svelte:42-45` pills gone; `Sidebar.svelte:44/:99`; `Dock.svelte:126/:128`; `Toasts.svelte:12` ("Undo · u"); `Inspector.svelte:22-23` (× icon button, tooltip `Close · esc`); `ChatInput.svelte:73` (placeholder).

**Keyboard shortcuts sheet** (`ShortcutsSheet.svelte`, ⌘/): groups and keys are **data**, not copy: `lib/shortcuts.js` exports `GROUPS` built from `CHORDS` + `LEADER` (`lib/keyboard.js`) for General / Workspace / Dock and static arrays for pane-local keys (Agent pane, Decisions & chat, Mission control, Board, Feature, Project, Diff, Weekly report, Team canvas, Design preview, Week planner, Settings lists) exactly as [audit.md §7](audit.md). Two-column layout (`04-keyboard-shortcuts-*.png`), search field in the title bar filters rows by label or key (case-insensitive, `⌘` matches "cmd"), group headers hide when empty, `Leader = ⌘;` line at the top. A unit test asserts every `CHORDS` / `LEADER` command with a label appears (§10) so the sheet cannot drift from the bindings. The Settings › Keyboard section and its placeholder are removed.

---

## 8. Removed duplicates: the final table

(From [audit.md §2](audit.md); each row = one home, the rest removed. File:line of the "removed from" are in the audit.)

| Action | Single home | Removed from |
|---|---|---|
| Resume (agent, every not-live state) | **strip** (§1.3.1); queued = Run now; blocked = disabled Resume | header `⏎ resume` (`AgentPane.svelte:139`), AgentInspector :49 |
| Kill, Delete…, Restart fresh, Shell here, Diff vs base, Open in kitty, Run off-hours | **`⋯` Menu** (header; inspector shows the same `AgentMenu`) | strips :184 :193 :194, AgentInspector buttons :51-55, header "More ⌘K" :141 |
| Interrupt, Suspend | header (live only) | AgentInspector :46-47 |
| Open session | AgentInspector | — |
| Status label | header `StatusBadge` | strip label :162/:165 (strip keeps only the reason) |
| WaitingChip | strip when a strip shows, else header | header while a strip shows |
| Permission decision | strip **and** the chat decision card (accepted: acts where the agent is / the inbox); strip says "also in Decisions" (link) | the `⌘⇧I` text |
| Zoom / restore, split, pane number, PaneFrame close | removed (single pane) | PaneFrame :33-39, Dock :126, Node :45-53 |
| Close tab | tab × (workspace, dock) + ⌘W | PaneFrame header |
| New session / Add project | each empty state; Sidebar `+` | duplicate pills |
| Hide dock | icon button (dock strip right end) | `⌘J hide` hint |
| Open Week planner | title-bar **PacingIndicator** popover button; banner button **Week planner** | StatusBar :35 pressure pill and :41 ☾ chip (information only), Settings › Pacing :203 + `p` key, banner `Details`/`Later` (renamed / dropped) |
| Dismiss banner | × | `Later` |
| Pause all | banner `red` + palette; Planner `P` (other screen) | — |
| Feature → Board | FeaturePane header | Tasks card `Open board` :282 |
| Project Diff | header ("Diff main") and per worktree (different targets) | — |
| BoardPane keys | Menu (right-aligned keys) | footer legend :317 |
| Board "Open task" | renamed **Open feature** (and fix the bug, §9) | |
| Mission control filter | workspace project context | Filter button, toolbar hint, `ProjectFilter`, empty-state hint |
| Mission row actions | row buttons | toolbar legend :299 |
| Team canvas Connect/Delete | inspector buttons | legend :559 |
| RunInspector PR | `PrLine` | `Open PR ↗` :66 |
| DesignSection keys | tooltips; preview gets a visible × | legends :207 :229 |
| Project settings Save | **per section** (Task backend, Notion each: Save + Cancel at the section foot) | the shared `save()` :293/:312 |
| Integrations Remove / Replace | row buttons | `SecretField` Remove/Replace :65-67 |
| MCP Reconnect | row link | drawer hint button :356 |
| MCP registry per project | Project settings › MCP overrides (all deep links with a project) | Settings › MCP with `projectId` |
| Allocation plan | **Edit weekly plan…**; Cancel | "Set up the weekly plan" / "Keep in the plan" |
| Overwatch on/off | switch in Settings › Overwatch | (palette + ChatInput link remain as entries) |
| Weekly report | Planner button | `report.list` palette bug (§9) |

Final per-surface action lists to implement are [audit.md §8](audit.md) (as amended by the lead on 2026-11-09): dock counts are open tabs; a not-live permission strip shows Resume instead of Answer in the pane; no project highlighted in All projects.

**Panes with no frame** (Team canvas, Diff, Report, Setup, dialogs): apply the same rules without redrawing: delete legends and key pills (audit §4 rows), native selects → Select (§2.1.1), Connect/Add menus → Menu, `esc cancel` text → tooltips, Cancel/Submit buttons without pills, dialogs in the `FloatingWindow` shell (§3.3), status icons from the `StatusBadge` vocabulary.

## 9. Behaviour bugs found by the audit (fix with the change that touches them)

1. Project settings: both Save buttons save both sections (`ProjectSettingsPane.svelte:293/:312`): one `save()` per section.
2. `report.list` palette command opens the current report instead of the list (`app.js` `openReport` callers; pass `'list'`).
3. Board menu "Open task" opens the feature, not the task (`BoardPane.svelte:296`): label **Open feature**.
4. Allocation "Edit the plan in the Week planner" opens the plan dialog (`AllocationSettings.svelte:243`): label **Edit weekly plan…**.
5. Planner **Pause all** feedback differs by button and key (`PlannerPane.svelte`): one handler, one toast.
6. Team canvas Duplicate: `D 1/2` on buttons vs `⇧D` in the legend (`TeamCanvasPane.svelte:621-622`): the button is `Duplicate`, key `⇧D` in the sheet and tooltip.
7. Four model lists with different contents: `lib/options.js:MODELS` (§2.1).

---

## 10. Tests (`ui/test/*.test.js`, vitest + `@testing-library/svelte`; mock `lib/ipc.js` as `dockview.test.js` does)

New files:

| File | Covers |
|---|---|
| `select.test.js` | Select: opens on click/↓/Enter/Space; ↑↓ Home End; type-ahead; Enter/Space picks; Esc closes keeping the value; disabled options skipped + reason tooltip; search appears at > 8 options and filters, "No … matches" line; error state sets `aria-invalid`; flips above at the bottom (mock `getBoundingClientRect`); roles/ARIA attributes; hidden input value; `onchange` only on user pick. |
| `menu.test.js` | Menu: items, key right-aligned, destructive/disabled/checkable/submenu roles, ↑↓ →/←, Enter runs + closes, Esc returns focus to the trigger, trigger tooltip has no key. |
| `tabstrip.test.js` | no shrink (`flex: none`, min-width content, max 280); `‹ ›` only when overflowing, disabled at each end (mock `scrollWidth/clientWidth/scrollLeft`); click scrolls by width − one tab; active/new tab scrolled into view; roving tabindex and ← → Home End; × on hover/active only; middle-click closes. |
| `floatwindow.test.js` | FloatingWindow: backdrop click and Esc close, Esc consumed by an open Select first; focus trap cycle; focus restored to the opener; one window at a time; width clamp (944 at 1024, never < 720); `role="dialog" aria-modal`. |
| `windows.test.js` | `openSettings/openProjectSettings/openPlanner/openShortcuts` set `app.window` and never `addPane`; Settings open + `openSettings('mcp', 's')` updates in place; `openSettings('mcp', s, projectId)` ⇒ Project settings on `mcp`; ⌘, toggles; replacing keeps the original `windowReturn`; a saved `settings` pane is dropped by `restoreLayout`; closed windows are not in `reopenClosed`. |
| `context.test.js` | `paneProject` for each type (§5.3); `visibleTabs` filter; active tab fallback on context change; opening another project's pane switches the context but not from `all`; `setContext` persisted; unknown saved project → `all`; ⌘n picks the n-th **visible** tab; sidebar project click sets the context; no highlight in `all`. |
| `dockviews.test.js` (extend `dockview.test.js`) | counts = open tabs of the kind (not filtered); `dockTabs` per view/filter; waiting dot rule (other view, other project); opening an agent/shell switches the view; filter switches to the opened agent's project when the agent would be hidden by another project's filter; per-view active tab restored; empty-view states (no `+` in Shells empty; "No agent running in <project>"); closing the last tab of a view switches or closes; `dock.next/prev` cycle within view+filter; ⌘J with no tabs opens a shell. |
| `shellcwd.test.js` | OSC 7 handler decodes `file://host/%20path`, ignores a foreign host; poll updates title on change only, stops when hidden or after OSC 7; left truncation at ~32 chars with `~`; command shells keep `spec.title`; OSC 0/2 used only when path-like; tooltip has the full path. Core: `terms.rs` test for `Term::cwd()`. |
| `persist.test.js` | `saveLayout` value contains `sidebarTree`, `workspace`, `dock.view/agentProject/activeAgent/activeShell`; `restoreLayout` defaults and pruning (§5.4); old layout with splits restores as one leaf in reading order with the focused tab active; the sidebar caret keys. |
| `shortcuts.test.js` | every `CHORDS`/`LEADER` command with a label is in `GROUPS`; removed ids are absent; search filter; no key listed that has no binding (compare with `keyboard.js`). |
| `hints.test.js` | render the title bar, banner kinds, status bar, sidebar, dock, agent strips, a Settings section: **no `.kbd` is a sibling of or inside a `button`/`a`** (except in `[role=menuitem]`, palette rows, the leader overlay, `.empty` line); the leader overlay shows only while `app.leader`. |
| `noselect.test.js` | `readdirSync` all `ui/src/**/*.svelte`: no `<select`. |

Changed files:

| File | Change |
|---|---|
| `keyboard.test.js` | tables for removed leader keys (`v s h l z` → `'cancel'`), kept `j k` / `]` `[` / ⌘1-9 / ⌘/ → `shortcuts`; `hint()` has no `pane.zoom`/`pane.split.*`. |
| `layout.test.js` | drop split/neighbor/resize/rects cases; add `flatten` (reading order, focused active, empty tree). |
| `dock.test.js` | remove zoom/ZOOMED/restore cases and `dockFills` zoom; add Hide dock icon button (tooltip), no `Kbd`; shells use `+` only in the Shells view. |
| `sidebar.test.js` | carets persisted via `app.sidebarTree`; project click sets the context. |
| `mission-pane.test.js` | no Filter button / ProjectFilter; follows `app.workspace.project`; empty state button "Show all projects". |
| `components.test.js`, `surfaces.test.js` | agent strip table of §1.3.1 (every state, live vs not live), header has no Resume/hints, `⋯` Menu items; AgentInspector shows Open session + `⋯`; banner kinds and buttons (audit §3); StatusBar information-only; PacingIndicator popover opens the planner window. |
| `term.test.js`, `termhost.test.js` | OSC 7 / title hooks wiring. |

Rust: `cargo test -p orch-app` (the `term_cwd`/`Term::cwd` test above; menu items include Help › Keyboard Shortcuts and the renamed tab items: extend the existing `menu.rs` test if any).

## 11. Suggested slicing for the implementation feature

1. **Foundations** (no visible change): `Tooltip`, `Select`, `Menu`, `lib/options.js`, `FloatingWindow`, `TabStrip` with tests (§10 first five).
2. **Select sweep**: the 27 `<select>` + `ModelSelect` + Files picker; `noselect.test.js`.
3. **Hints and duplicates**: audit §2/§4 per file (agent pane + inspector first), `ShortcutsSheet`, `hints.test.js`.
4. **Windows**: Settings/Project settings/Planner/Shortcuts, `openSettings` rewrite, deep-link rewrites (§3.2), menu.rs Help.
5. **Single pane + context**: `layout.js` flatten, `Workspace` + `TabStrip`, delete `Node`/`PaneFrame` header/zoom/split commands, `restoreLayout` migration, Mission control context, sidebar/icon-rail highlight.
6. **Dock views + persistence**: `Dock.svelte` views/filter, `sidebarTree`, `persist.test.js`.
7. **Shell live title**: `term_cwd` (core) + OSC 7 + poll.

## 12. Lead decisions after T-293 (they override the sections above where they differ)

1. **`openProjectSettings(projectId, section = null, server = null)`**: a third argument, so the MCP deep links rewritten from `openSettings('mcp', server, projectId)` (§3.1, §3.2) keep the selected server: `app.window = { kind: 'project-settings', projectId, section, server }`, and Project settings › MCP overrides passes `selected={spec.server}` to `McpSettings` (today `null`, `ProjectSettingsPane.svelte:314`). Test in `windows.test.js`.
2. **Panes with no project** (`paneProject(spec) == null`: an editor opened outside a project, a team canvas opened from All projects, the weekly report, Mission control, Setup) are app-wide: always visible, they never change the context (§5.3).
3. **Dock project Select waiting dot**: the closed project Select shows the waiting dot when an agent filtered out by `dock.agentProject` needs the user (`counts(p).needs_you` of the other projects); the view Select keeps its dot for agents outside the current view. Each open option keeps its per-project dot (02-dock-strip).
4. **Dock restore**: `dock.view` = saved value, else the kind of the saved `dock.active`; an `activeAgent` outside the filter falls back to the filter's first agent tab, else the empty view (api-contract).
5. **Agent inspector has no Resume** (audit §2.1): resume through Open session, then the strip, or the palette.
6. **Sidebar**: carets on project and feature rows only; agent rows are leaves.

## 13. Round 2 (user review of the PR; spec §7, R1–R15)

Frames of this round are in the same `design/` folder; the rules above still apply. Settings content per section is in [settings-brief.md](settings-brief.md); the flows are in [flows.md](flows.md) and `06-flow-*.png`.

### 13.1 Review fixes (R1–R13)

| R | Frames | Implementation |
|---|---|---|
| R1 Overwatch column handle | every frame with the chat column | the handle of `allow-overwatch-menu-resizing` (#47) stays; same visual rule as the dock handle. No change to its code; do not draw anything over it |
| R2, R3 worktree row | `04-feature-pane`, project pane frames | `ProjectPane.svelte`: Diff → icon button (± glyph, tooltip "Diff"), Remove… → bin icon button, `color.danger`, tooltip "Remove worktree…" (same `confirm` flows) |
| R4 headers | project pane | one header pattern for every pane: identity left, icon buttons right, `⋯` Menu last. Project: gear (Project settings), bin in danger colour (Remove project…), `⋯` = Fetch, Shell, Diff main, Trust in a pane (untrusted only) |
| R5 title bar Decisions | `03-title-status-banner` | one gap token between icon, label and count |
| R6 tab overflow | `03-tab-overflow-*` | the strip clips at the left edge of ‹ ›: one fade on the clipped tab, nothing drawn between it and the buttons, buttons on an opaque background (TabStrip, §2.3) |
| R7 suspended menu | `04-agent-suspended`, `04-agent-suspended-menu-closed` | `⋯` trigger drawn pressed while its Menu is open, Menu right-aligned under it |
| R8, R12 frame placement | Drive file | design only (frames moved to the top row of their pages) |
| R9 feature header | `04-feature-pane` | icon buttons Pause/Resume, Stop, Delete (danger) with tooltips, then `⋯` = Board, Canvas, Publish/Refresh in Notion, Open in Notion. `FeaturePane.svelte` header; the Tasks card keeps no Board button |
| R10 doc path | `04-feature-pane` | under the doc tabs a muted line: repo-relative path of the shown doc (only when it is in the repo) with an Open in nvim icon button at its end; the old text button goes |
| R11 Select height | all | Select (regular) = text input height (`--size-control`); `compact` only in the tab strip and the dock strip. Check every row where a Select sits next to an input (feature message box To + input, Settings rows) |
| R13 PR line | `04-feature-pr-running`, `04-feature-pr-review-done` | `PrLine` in the feature pane's team column: link, CI dot and `n/m` count, checks popover (list of checks with state, "Open checks on GitHub"); running and review-done states |

### 13.2 Settings sections (R14)

Each section is a frame `04-settings-<section>-1280.png` / `04-project-settings-<section>-1280.png` (plus `-1024`, and state variants named in the file). Anchors, all in `ui/src/components/` unless noted; apply [settings-brief.md](settings-brief.md) §0 rules to every one (cards with a header, label column, help text, no `Kbd`, autosave vs footer):

| Frame | Component | Notes |
|---|---|---|
| `04-settings-claude-code-*` (+ `update-available`, `update-failed`) | `ClaudeSettings.svelte`, `ClaudeUpdateBlock` | groups Installed / Update / Compatibility / Account / Defaults for new sessions (**new**, optional) |
| `04-settings-integrations-*` (+ `editing-a-key`) | `IntegrationsSettings.svelte` | cards, full texts, needs-you first |
| `04-settings-overwatch-1280` | `OverwatchSettings.svelte` | gains a Notifications group: desktop and sound switches (`notifications`, today only the palette ids `notify.desktop` / `notify.sound`) |
| `04-settings-permissions-1280` | `PermissionsSettings.svelte` | policy files (edited in the file), no-rule-matches Selects, hold seconds (0 = don't hold) |
| `04-settings-mcp-servers-*` (+ `server-error`) | `McpSettings.svelte` | master/detail |
| `04-settings-terminal-editor-*` (+ `no-kittyconf`) | **new** `TerminalSettings.svelte` (replaces the "Not designed yet" branch of `SettingsPane`/the window) | Appearance (theme Select, kitty colours switch: `setTheme`, `setKittyTheme`), Terminal (read-only from `app.kitty`, Open kitty.conf), Renderer (**new** setting), Editor command (**new** unless read from kitty.conf), Shells (**new** Select) |
| `04-settings-pacing-policy-*` (+ `governor-in-red`) | `PacingSettings.svelte` | groups Live agents / Limits / Off-hours / Catch-up |
| `04-settings-memory-*` (+ `warn`) | **new** `MemorySettings.svelte` | stacked bar from the status bar's memory sample, pressure badge, warn threshold (**new**), explanation text, links to Pacing and Renderer |
| `04-settings-builds-1280` | `BuildsSettings.svelte` | PR builds list, update channel |
| `04-project-settings-general-*` (+ `untrusted`) | **new** `ProjectGeneral.svelte` (the "Not designed yet" branch of `ProjectSettingsPane`) | rename, path actions, default branch Select, fetch, trust, worktrees, danger zone. Name and default branch are the only dirty fields (footer Save/Revert) |
| `04-project-settings-files-*` (+ `missing-file`) | `FilesSettings.svelte` | |
| `04-project-settings-task-backend-*` (+ `linear-not-connected`) | `ProjectSettingsPane.svelte` backend branch | |
| `04-project-settings-intake-1280` | `IntakeSettings.svelte` | |
| `04-project-settings-allocation-1280` | `AllocationSettings.svelte` | |
| `04-project-settings-notion-1280` | `ProjectSettingsPane.svelte` notion branch | |
| `04-project-settings-mcp-overrides-1280` | `McpSettings.svelte` with a project | |

"**new**" rows are listed in [settings-brief.md](settings-brief.md) §4: build only those the team accepts, and hide the rest; none changes another screen.

### 13.3 Flows (R15)

`06-flow-<id>.png`, one storyboard per flow of [flows.md](flows.md) (start, context, tabs, dock, agent-state, decision, feature, project, settings, project-settings, planner, shortcuts, terminal, migrate). They carry no new rule: each caption names the rule it shows. Use them as acceptance walkthroughs: each flow is one manual test of the implementation, and F2 to F5, F9, F10 and F14 are also covered by tests (§13.4).

### 13.4 Tests added by round 2 (`ui/test/*.test.js`)

- `settings-sections.test.js`: both windows list their sections (9 and 7), each renders without the "Not designed yet" text, no `Kbd` inside the nav, deep link opens on its section.
- `terminalsettings.test.js`: theme Select calls `setTheme`; kitty colours switch calls `setKittyTheme`; missing `app.kitty` shows the no-kitty.conf text.
- `memorysettings.test.js`: bar segments from the sample, pressure badge per level, warn threshold validation.
- `projectgeneral.test.js`: rename saves, default branch change, Revert, untrusted state shows Trust in a pane.
- `headers.test.js`: project and feature pane headers: icon buttons have tooltips (`aria-label`), danger icons use the danger class, `⋯` is last and holds the listed items.
- `prline.test.js`: running and review-done states, checks popover rows.
- `docpath.test.js`: path line only for docs inside the repo.
- Extend `selectheight` in `noselect.test.js`: a Select next to an input gets the input's height class.
