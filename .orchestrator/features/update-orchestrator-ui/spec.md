# Update Orchestrator UI · Feature spec

> Feature `update-orchestrator-ui` · Integration branch `orch/update-orchestrator-ui/integration` · Team: lead, back, front, ui, ux.
> Companions: [api-contract.md](api-contract.md) (backend additions and state keys), [ux-brief.md](ux-brief.md) (how the design is used and reviewed).

## 1. Problem

The redesign `improve-orchestrator-ui-ux` (PR #50) is merged on main as **design only**: an audit, FigCraft frames, PNG exports and an implementation hand-off. None of it is in the code. This feature implements it.

**Source of truth** (on main, brought into this branch by `rebase_base` before any task starts):

- `.orchestrator/features/improve-orchestrator-ui-ux/design-notes.md`: the hand-off. Its §12 (lead decisions) and §13 (round 2) override the earlier sections where they differ.
- `.orchestrator/features/improve-orchestrator-ui-ux/audit.md` (every action and `Kbd`, per file; §8 = actions per screen after cleanup), `settings-brief.md` (settings sections), `flows.md` + `design/06-flow-*.png` (acceptance walkthroughs), `api-contract.md` (state keys), `design/*.png` (looks).
- Rule of precedence (design-notes header): the PNG wins for looks, the notes win for behaviour.

The design was done at main 5882f8b (+ Intake #48). Line numbers in the notes drift: anchor on function names.

## 2. Goals

What the user sees once this ships (design-notes §0):

1. No `Kbd` pill next to or inside a control; keys live in tooltips, the palette, Menu items, the **Keyboard shortcuts** sheet (⌘/, Help menu), one empty-state line and the leader overlay.
2. One home per action on every screen (agent strip owns Resume, header `⋯` Menu owns the rest, banner is app-wide only, Mission control follows the project context).
3. App Settings, Project settings, Week planner and Keyboard shortcuts are **floating windows**, never workspace tabs. Dialogs use the same window shell.
4. **Single pane**: no splits, zoom or pane numbers; one tab strip with a **project context** Select; tabs never shrink; `‹ ›` overflow.
5. **Dock = Agents / Shells** views, agents filterable by project; shell tabs titled by their **live working directory**.
6. A shared **Select** and **Menu** replace every native `<select>` (34) and every ad-hoc popover menu.
7. Sidebar tree, workspace project, dock view and agent filter **survive a restart**; old split layouts migrate to one tab strip.
8. Pane headers follow one pattern (identity left, icon buttons right, `⋯` last): project, feature, agent (R2–R4, R9, R10); the feature pane shows the PR line (R13).
9. Every Settings and Project settings section is redrawn to `settings-brief.md` §0, including the three placeholders (Terminal & editor, Memory, Project › General).
10. The behaviour bugs of design-notes §9 are fixed.

## 3. Scope decisions (lead)

### 3.1 Rows that exist only in the design (settings-brief §4)

The design leaves the choice to this feature. Decision (the user can change it at spec approval):

| Row | Ships? | Why |
|---|---|---|
| Overwatch › Notifications switches (desktop, sound) | **yes** | the `notifications` setting exists; UI only |
| MCP overrides › Inherit / On / Off per server | **partly**: two options **Inherit** / **Override** (a project-scoped copy with its own env); **Off hidden** | UI only, on top of the existing project/global scope. The store has no per-project "disabled" state, so Off would need a backend and runtime change (out of scope). Without Off, "On" would read like Inherit, hence the label Override (lead, during T-316) |
| Project › General › Name (rename) and Default branch | **yes** | columns exist; back adds the write path (api-contract §2) |
| Memory › Warn above (MB) | **yes** | FR-904 asks for a warning threshold; a UI setting `memory_warn_mb` (api-contract §3), default 1600 MB, validated by the UI |
| Claude Code › Defaults for new sessions | no (hidden) | new app-wide fallback model; separate feature |
| Terminal & editor › Renderer, Editor command / Reuse the open nvim, New shell opens in | no (hidden) | new settings + renderer switch; separate feature |
| Memory › Show in the status bar | no (hidden) | the status bar already shows the sample |
| Project › General › Remove merged worktrees automatically | no (hidden) | new background behaviour |

The Memory stacked bar shows what the existing memory sample carries (per agent when the sample has it, else one total bar); no new sampling.

### 3.2 Shell live title

design-notes §6.1, as decided there: a UI poll of the core command `term_cwd` (the shell process's cwd, every 1.5 s while the window is visible), OSC 7 honoured when a shell sends it, OSC 0/2 only when path-like. No shell integration injected.

### 3.3 Out of scope

New features beyond §3.1, tokens/theme rework, the terminal look, Mission control's layout beyond the dedup rules, the chat column's content, drag-reordering tabs, a Menu on sidebar rows, settings search.

## 4. Work split

Ordered as design-notes §11. The frontend is one long chain on the `front` node, so branches never fight over `lib/app.svelte.js` and the settings components; `back` runs in parallel.

| # | Task | Owner | Depends on |
|---|---|---|---|
| T1 | Backend: `term_cwd` + shell name, menu (Tab n, Close Tab, Help › Keyboard Shortcuts), project rename + default branch | back | — |
| T2 | Foundations: `Tooltip`, `Select`, `Menu`, `lib/options.js`, `FloatingWindow`, `TabStrip` + their tests (no visible change) | front (opus) | — |
| T3 | Select sweep: the 34 `<select>` + `ModelSelect` + Files access picker; `noselect.test.js`; one model list (§9.7) | front | T2 |
| T4 | Floating windows: Settings, Project settings, Week planner, Shortcuts sheet (`lib/shortcuts.js`), openers and deep links (§3), ⌘/ and AZERTY, dialogs in the window shell, Discard-changes guard | front (opus) | T3 |
| T5 | Single pane + project context: `layout.flatten` migration, `Workspace` + `TabStrip`, remove split/zoom/pane numbers/`Node`/`PaneFrame` header, Mission control follows the context, sidebar/icon-rail highlight, `sidebarTree` + `workspace` persistence | front (opus) | T4 |
| T6 | Dock views + filter + waiting-dot rule (§12.7), dock persistence, shell live title (OSC 7, poll, title format) | front | T5, T1 |
| T7 | Hints and duplicates: audit §2/§4/§8 per file, AgentMenu, agent strips and inspector, pane headers (R2–R4, R9, R10), PrLine (R13), title bar / status bar / banner, §9 bugs, `hints.test.js`, `headers.test.js`, `prline.test.js`, `docpath.test.js` | front (opus) | T6 |
| T8 | Settings sections (R14): every section of both windows per settings-brief, new `TerminalSettings`, `MemorySettings`, `ProjectGeneral`, the §3.1 rows that ship (hidden rows not rendered), round-2 tests | front | T7, T1 |
| T9 | Acceptance walkthrough: flows F1–F14 and audit §8 against the integration branch; list deviations (file, frame, rule) | ux | T8 |

`ui` reviews each front task against the PNGs when front asks (`request_review`); ui owns no code. No new frames are planned; if a task finds a gap in the design, lead decides, and ui draws only if lead asks.

## 5. Acceptance

1. Every test of design-notes §10 and §13.4 exists and passes; `make test` and `make lint` pass (socket/pty tests are judged by CI).
2. `grep -rn "<select" ui/src` returns nothing; `grep -rn "<Kbd" ui/src` hits only the allowed places (§7 of the notes), enforced by `hints.test.js`.
3. No code path calls `addPane` for settings, project settings or the planner; a saved layout with such panes or with splits restores as one tab strip.
4. Each flow of `flows.md` walks through as drawn (T9 report has no open deviation, or each one is accepted by lead with a reason).
5. The §3.1 rows that ship work end to end (saved and read back); the hidden ones are not rendered.
6. The bugs of design-notes §9 are fixed and covered by a test where one is listed.
