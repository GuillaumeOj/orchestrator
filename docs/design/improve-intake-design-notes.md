# Improve Intake design notes: Work browser, programs, tracker settings

UX pass for improve-intake, 2026-11-13, by `ux` for `ui`, then the build feature (`improve-intake-build`). Sources: [spec](../../.orchestrator/features/improve-intake/spec.md) (approved, §6 Q1–Q6 accepted), [UX brief](../../.orchestrator/features/improve-intake/ux-brief.md), [API contract](../../.orchestrator/features/improve-intake/api-contract.md). Earlier notes stand unless a section here says **supersedes**: [P4.5](p4-5-design-notes.md) (**P4.5 §n**: Ready lane §1–§4, prefilled dialogs §5, origin chip §6, Intake settings §7, keys §10) and [P4.2](p4-2-design-notes.md) (**P4.2 §n**: lanes §1.1, rows §1.3–§1.5, details §1.8, keys §3).

Copy rules as before: sentence case, no exclamation marks, name the item / feature / project, local 24 h times, **every number says what it is**. Tracker names in copy: **GitHub**, **Linear**, **Local**. Item text (titles, bodies, labels) is untrusted: plain text only (P4.5 §3), only "Open in GitHub ↗" / "Open in Linear ↗" navigates.

Section numbers follow ux-brief §2 (1–12). Fixtures are in §0.2; every board uses them so the boards read as one story. Widths: shell 1512 (chat column 360) and 1280 (chat column 340), P4.3 §9.

**Two visual rules for ui**
1. **Tracker's words vs app's state never share a colour.** Upstream state (open, closed, Linear state, issue type, labels) is **monochrome**: shape of the glyph + `text.muted` / `text.primary` only. The **app-state chip** is the only coloured thing on a row and uses the existing status tokens (`status.queued`, `status.working`, `status.at-gate`, `status.done`). The "ready" mark keeps the P4.5 rule: dashed outline, no fill, no accent.
2. **Tracker items and team tasks never look alike.** Team tasks appear in the Work browser only inside the collapsed "Team tasks" fold, as compact read-only lines (no glyph of the tracker, no app-state chip, no start actions); their real home is the Feature pane (§9).

---

## 0. Fixtures and summary

### 0.1 Decisions taken in these notes (summary)
| Topic | Decision | § |
|---|---|---|
| Work browser layout | Outline tree (left) + details panel (right), one pane, tab in the workspace | 2 |
| Board view of the same scope | **Not drawn.** Upstream status filter + Ready lane cover it; see 2.8 | 2.8 |
| Depth > 4 | Indent stops at level 4; deeper rows show a level tag `L5`; **Focus here** (`z`) re-roots the tree with a breadcrumb | 2.4 |
| Solo key | `a` (as P4.5 Ready rows); `s` stays Suspend everywhere | 12 |
| Mission control key | `w` opens Work (scoped to the filter); leader `⌘; b` opens it from anywhere | 1, 12 |
| Programs in Mission control | New lane **Programs** between Running and Waiting on | 7.1 |
| Program plan graph | **List with dependency marks**, plus a small read-only graph strip only when there are edges | 6.3 |
| Batched spec approval | **Not recommended** for v1; spec cards get a program tag and "next spec" chaining instead | 7.5 |

### 0.2 Fixtures

**Projects.** `shop-api` (GitHub `acme/shop`, Projects v2 board none, Ready filter label `ready`), `mobile` (Linear team `ENG`, every project, Ready filter "unstarted in the current cycle"), `blog` (Local, backlog `.orchestrator/backlog/*.md`, 2 files). Week room: shop-api 12 pp left (its allocation).

**GitHub fixture `acme/shop`** (milestone grouping on; one open milestone `v2`)

| Node | Type | Upstream | Labels | Children | Dependencies | App state |
|---|---|---|---|---|---|---|
| Milestone **v2** | milestone | open, due 1 Dec | | 2 issues | | rollup "1 running · 2 waiting · 1 merged" (ready is not an app count, §2.6) |
| └ **#12 Checkout v2** | Epic | open | `checkout` | 3 sub-issues, 0 done | | program root: chip "program · 0 of 3 merged" (roll-up in details, §2.6) |
| &nbsp;&nbsp;└ **#13 Address step** | Feature | open | `checkout`, `ready` | 2 sub-issues, 1 done | blocking #14 | **running · checkout-address · executing** |
| &nbsp;&nbsp;&nbsp;&nbsp;└ #16 Postcode lookup | Task | closed (completed) | | | | done (merged PR #58) |
| &nbsp;&nbsp;&nbsp;&nbsp;└ #17 Address validation errors | Bug | open | | | | none |
| &nbsp;&nbsp;&nbsp;&nbsp;└ *Team tasks · 4 · feature checkout-address* (fold) | | `orch` sub-issues #70–#73 | | | | |
| &nbsp;&nbsp;└ **#14 Payment step** | Feature | open | `checkout` | 1 sub-issue | **blocked by #13** | **queued · Checkout v2 · waiting for checkout-address to merge** |
| &nbsp;&nbsp;&nbsp;&nbsp;└ #18 Saved cards | Task | open | | | | none |
| &nbsp;&nbsp;└ **#15 Order summary** | Feature | open | `checkout` | | | **queued · Checkout v2** (Governor live cap 2/2: checkout-address and welcome-tour run) |
| └ #9 Flaky checkout e2e | Bug | open | `ready`, `ci` | | | ready |
| (no milestone) **#21 Refunds on partial orders** | Feature | open | `ready`, `billing` | | | PR #61 open · feature partial-refunds |
| (no milestone) #20 Old cart cleanup | Task | closed (not planned) | | | | none (hidden by "hide closed") |

Program **Checkout v2** (from #12, grouping per child, max 2 at once): pf-1 `checkout-address` (#13, team default-web, ≈ 4–7 pp) · pf-2 `checkout-payment` (#14, team, depends on pf-1, ≈ 5–8 pp) · pf-3 `checkout-summary` (#15, solo, ≈ 1–2 pp). Total ≈ 10–17 pp week; room 12 pp → over room (high end).

**Before-program variant** (for the plan dialog boards): same tree, #13 / #14 / #15 app state none, #13 has the `ready` label → ready.

**Linear fixture team `ENG`** (no initiatives)

| Node | Kind | Upstream | Est. | Children | Dependencies | App state |
|---|---|---|---|---|---|---|
| **Mobile onboarding** | Linear project | In progress · target 1 Dec 2026 | | 2 milestones, 5 issues | | rollup "1 running" |
| └ **M1 · Sign-up** | project milestone | 0 of 2 done | | 2 issues | | |
| &nbsp;&nbsp;└ **ENG-101 Email sign-up** | issue | Todo (unstarted) · High · Cycle 14 | 3 | 2 sub-issues | blocks ENG-102 | ready |
| &nbsp;&nbsp;&nbsp;&nbsp;└ ENG-104 Verify email link | sub-issue | Done (completed) | 1 | | | none |
| &nbsp;&nbsp;&nbsp;&nbsp;└ ENG-105 Resend email | sub-issue | Todo | 1 | | | none |
| &nbsp;&nbsp;└ **ENG-102 Apple sign-in** | issue | Backlog | 5 | | **blocked by ENG-101** | none |
| └ **M2 · First run** | project milestone | 0 of 2 done | | 2 issues | | |
| &nbsp;&nbsp;└ **ENG-110 Welcome tour** | issue | In Progress (started) | 3 | | | **running · welcome-tour · spec** |
| &nbsp;&nbsp;└ **ENG-111 Sample project** | issue | Todo · Cycle 14 | 2 | 1 sub-issue | | ready |
| &nbsp;&nbsp;&nbsp;&nbsp;└ ENG-112 Seed data | sub-issue | Todo | 1 | | | none |
| └ ENG-120 Analytics events (no milestone) | issue | Todo · Cycle 15 | 2 | | | none |
| **Billing revamp** | Linear project | Planned · no target | | 9 issues | | collapsed |

**Local fixture `blog`**: `dark-mode.md` (ready), `rss-feed.md` (none). Flat.

**Deep fixture** (state boards only): `acme/shop` #40 Platform migration → #41 → #42 → #43 → #44 → #45 Rename env vars (level 6).

---

## 1. Information architecture (point 1)

### 1.1 Three contexts after the change
| Context | Shows | Does not show |
|---|---|---|
| **Mission control** (all projects, filter) | What is running, waiting, queued, **programs**, Ready to start (short list of the Ready filter), Done today | Tracker browsing (→ Work), team tasks |
| **Project** (Project pane, sidebar project row) | Project's runs, PRs, settings; a **Work** entry scoped to the project | Team tasks of every run |
| **Feature** (Feature pane) | Its source item (header), its **team tasks** (compact board, §9), timeline, docs | Other tracker items |
| **Work** (new pane) | Tracker items as a tree, with app state; actions that start work | Team tasks except inside the fold, runs' detail |

### 1.2 Where Work opens from
| Entry | Scope | Copy |
|---|---|---|
| Mission control header, button left of the filter | the filter's projects (all when no filter) | Button "Work" with key hint `w`; tooltip "Browse GitHub, Linear and local work of the shown projects" |
| Mission control key `w` | same | — |
| Ready lane header, **Browse all** (§8) | that project (one project shown) / the filter's projects | "Browse all" |
| Project pane, header button | that project | "Work" · tooltip "Browse shop-api's work in acme/shop" |
| Sidebar, project row `⋯` menu and the project's sub-row **Work** | that project | "Work" |
| Palette (⌘K) | "Open Work (all projects)" · "Work: shop-api" per project · "Find work item…" (opens Work with `/` focused) | |
| Leader `⌘; b` | context project if any, else all | Shortcut sheet row "Work browser" in Workspace |
| Feature pane source header ↗ "Show in Work" | the feature's project, tree revealed at the source item | |

One Work tab per scope: opening Work with the same scope focuses the existing tab; a different scope opens a new tab titled "Work · shop-api" / "Work · 3 projects" / "Work · all projects".

### 1.3 What replaces each Board entry point (spec §3.5; supersedes the Board pane)
| Before (HEAD) | After |
|---|---|
| Feature pane `b` and its `⋯` item "Board" | `b` scrolls to and focuses the **Tasks** section of the same pane (§9); `⋯` item renamed "Show source in Work" (`w`) |
| Run inspector button "Open board" | "Show tasks" (opens the Feature pane at Tasks); second button "Show in Work" when the run has a source item |
| Palette "Open the Board of the focused project" / "Board: <project>" | "Open Work (shop-api)" / "Work: shop-api" |
| Mission control `sync_conflict` outcome → Board on the task | Feature pane of the task's run, Tasks section, task focused and flashed (§9.4) |
| Waiting chip `T-12` → Board on the task | same as above |
| Chat link to a task (`o.task`) | same as above |
| Shortcut sheet group "Board" | removed; group "Work browser" (§12) added; Feature group `b` relabelled "Tasks" |
| Project settings › Task backend, Linear column mapping | moved into Project settings › Tracker (§10) |

**Removal notice**: the first time after the update that the user presses an old Board path (palette entry is gone, so this is the Feature `b` / inspector), a one-time toast: "The Board is gone: tracker items are in Work (w), team tasks are here in Tasks." Shown once (UI preference `work.board_notice_seen` via `settings_set`). No other migration UI. Boards on page 04.4 get the sticker "Removed in improve-intake".

---

## 2. Work browser (point 2)

### 2.1 Pane anatomy (1512)
```
┌ Work · 2 projects ───────────────────────────────────────────── updated 4 min ago · Refresh r ┐
│ [/ Search work…                    ] [Filters f · 2] [Hide closed ✓] [Only ready] [⋯]            │
│ ─ tree (≈ 640) ───────────────────────────────────────────┬─ details (420) ──────────────────── │
│ ▾ ⌂ shop-api · GitHub acme/shop                    3 ready │ acme/shop › v2 › #12 Checkout v2     │
│   ▾ ⚑ v2 · due 1 Dec                         2 issues      │ ◯ #13 Address step                   │
│     ▾ ◯ #12 Epic Checkout v2   checkout   0/3  [program]  │ Feature · open · checkout, ready     │
│       ▾ ◯ #13 Feature Address step  …  1/2  [running ·…]  │ …                                    │
│ ▾ ⌂ mobile · Linear ENG                            2 ready │                                      │
│ ─────────────────────────────────────────────────────────┴───────────────────────────────────── │
│ hint row: ⏎ details · t team run · a solo · p program · o open · x select · / search           │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```
- **Header** (40 px): title "Work · <scope>", right: status text (§11) and **Refresh** (`r`).
- **Toolbar** (36 px): search field (§4), **Filters** button with the count of active filters, two quick toggles **Hide closed** (on by default) and **Only ready**, `⋯` (Group by milestone (GitHub) ✓, Expand all to level 2, Collapse all, Show hidden items, Project settings › Tracker).
- **Tree** (left, fills), **details panel** (right, 420 px at 1512, 360 at 1280; can be closed with `Esc` from the tree → tree takes the full width; `⏎` reopens).
- **Selection bar** (§5.4) replaces the toolbar while ≥ 1 item is selected with `x`.
- **Hint row** at the foot: only the keys the focused row allows (P4.2 §1.9 rule).
- **Project level**: when the scope has several projects, a level-0 row per project: `⌂ shop-api · GitHub acme/shop` + right "3 ready" (count of ready, not hidden). One project in scope → no project row, the tree starts at its roots.
- **Opening state**: remembers per scope the expanded nodes and the focused node (session + restart); first open expands level 1 of each project (milestones / Linear projects) and nothing deeper.

### 2.2 Tree row (32 px, one line)
Slots left → right:

| Slot | Content | Rules |
|---|---|---|
| Indent | 16 px per level; level 0 = project row, roots are level 1 (one project in scope: no project row, everything shifts left one step, level numbers unchanged) | max 4 steps (§2.4) |
| Disclosure | ▸ / ▾, 12 px; blank for leaves | click or `h`/`l` |
| Kind glyph (16 px, monochrome) | see 2.3 | shape carries the upstream state for issues |
| Ref | `#13` · `ENG-101` · `v2` (none for milestones/projects: the title is the ref) · `dark-mode.md` | mono 12, `text.muted` |
| Issue type | `Epic` `Feature` `Bug` `Task` (GitHub issue types only) | 11 px, `text.muted`, no chip fill, 1 px `border.subtle`; absent when the org has no types |
| Title | `title` | 13 px, ellipsis, takes the space left; closed items: `text.muted` + strikethrough **off** (closed ≠ deleted) |
| Upstream state (Linear; GitHub only with a board) | "Todo" · "In Progress" · "Backlog"; GitHub with a Projects v2 board: its Status (`board_status`, "Todo") | `text.muted`, 12 px. GitHub open / closed is in the glyph shape only. |
| Labels | ≤ 2 neutral chips then "+n" | `orch*` and `size:*` never shown (P4.5 §2) |
| Dependency mark | `⛓ blocked by #13` (open blocker) · `⛓ blocked by 2` (several) · nothing when every blocker is closed | `text.muted`; tooltip lists blockers with state; `blocking` is only in details |
| Children progress | `1/2` (upstream done / total, direct children, team tasks excluded: F31) | mono 12, `text.muted`; for milestones / Linear projects "2 of 6" in the same slot |
| App-state chip | §2.5 | right-aligned, fixed column 200 px (1512), 160 (1280) |

Row states: rest · hover · focused (focus ring) · open in details (subtle fill) · selected (`x`: check box replaces the disclosure column's left 16 px, row tint `surface.selected`) · loading children ("Loading…" child line) · hidden (always in the tree, no "ready" chip; with "Show hidden items" on: 60 % text, tag "Hidden") · deleted upstream (§11).

**1280**: labels drop; the dependency mark becomes the glyph `⛓` only (tooltip keeps the text); issue type stays; app-state chip text shortens (§2.5 "short").

### 2.3 Kind glyphs (component **Kind glyph**, page 02, 16 px, `text.muted`; Nerd Font codepoints chosen by ui)
| Kind | Glyph | `aria-label` |
|---|---|---|
| Project (level 0) | house / repo outline | "Project shop-api" |
| GitHub milestone | flag | "Milestone v2" |
| GitHub issue, open | ring ◯ | "Open issue" |
| GitHub issue, closed completed | ring with check ◉✓ | "Closed issue, completed" |
| GitHub issue, closed not planned | ring with slash ⊘ | "Closed issue, not planned" |
| Linear initiative | target | "Initiative" |
| Linear project | cube | "Linear project" |
| Linear project milestone | diamond ◇ | "Project milestone" |
| Linear issue | by state type: backlog dashed circle, unstarted circle, started half circle, completed check circle, canceled x circle | "Linear issue, Todo" (the state name) |
| Local item | file | "Local item" |
| Team tasks fold | stacked squares | "Team tasks" |

The issue type is text (2.2), never a different glyph: types are org-defined and unbounded.

### 2.4 Depth beyond 4
- Levels count from the project row (0), so a node keeps its level in every scope (one project or several). Indent grows to level 4 (64 px) and stops. Rows at level ≥ 5 keep the level-4 indent and show a **level tag** `L5`, `L6` (mono 11, `text.muted`) before the glyph. A thin vertical guide line per level up to 4.
- **Focus here** (`z`, row `⋯`, details): re-roots the tree on the node. A breadcrumb bar replaces the toolbar's left part: `shop-api › #40 Platform migration › #41 … › #43` (middle items collapse to `…` past 3; each segment re-roots there, the first one (the project) leaves focus mode); `⇧Z` goes back up one level; `Esc` on the bar leaves focus mode. Search and filters apply inside the focused subtree; the header says "Work · shop-api › #43".
- Hint when a row at level ≥ 5 is focused: "z focus here".

### 2.5 App-state chip (component **App-state chip**, page 02)
| `app.state` / derived | Chip (long · short at 1280) | Token | Tooltip |
|---|---|---|---|
| none, `ready: true` | "ready" | dashed outline, no fill, `text.muted` (P4.5 rule) | "Matches the Ready filter (label ready)" |
| none | — (no chip) | | |
| queued, waiting_on ≠ [] | "waiting · Checkout v2" · "waiting" | `status.queued` outline | "In program Checkout v2: waiting for checkout-address to merge" |
| queued, waiting_on = [] | "queued · Checkout v2" · "queued" | `status.queued` | "In program Checkout v2: queued by the Governor (live cap 2/2)" |
| running | "running · checkout-address · executing" · "running · exec." | `status.working` | "Team run checkout-address, phase executing. g opens the feature." |
| solo | "solo · fix-feed" · "solo" | `status.working` | "Solo session fix-feed" |
| pr | "PR #61 open" · "PR #61" | `status.at-gate` | "Feature partial-refunds · PR #61 waiting for review" |
| done | "merged" · "merged" | `status.done` | "Merged 12 Nov 14:02 · feature checkout-address" |
| program root (node is a program's `root`) | "program · 0 of 3 merged" · "program 0/3" | `status.working` outline | "Program Checkout v2: 1 running, 2 waiting" |
| closed upstream with app none | — (the glyph says closed) | | |
| running/queued **and** closed upstream | chip as above + mark `closed upstream` (`text.muted`, warning glyph) | | "Closed in GitHub while the feature runs. It is not stopped." |

The chip is clickable (feature → Feature pane; program → Program view; PR → browser). Not a separate Tab stop.

### 2.6 Roll-up on parents
- Parents with app activity below show a **roll-up** instead of an app-state chip when they have no state of their own: text in the chip column, `text.muted`, no fill: "1 running · 1 waiting · 1 merged" (order: running, pr, queued+waiting as "waiting", done as "merged"; zero counts omitted; > 3 parts: "1 running · 3 more"). Tooltip lists every count with its noun.
- Children progress (2.2, upstream) and roll-up (app) are always separate slots: "1/2" vs "1 running".
- A parent with its own state (e.g. #13 running) shows its chip; the roll-up of its children goes to the details panel.
- Collapsed parents keep the roll-up (that is the point of it).
- Roll-up counts **descendants** (not only direct children), never team tasks (F2).

### 2.7 "Team tasks" fold
- Under the source item of a feature whose team wrote tasks back (`team_task: true` children), **one** line, last among the children, collapsed by default: `▸ ⧉ Team tasks · 4 · feature checkout-address` (`text.muted`, 28 px, no app-state chip, no progress).
- Expanded: one compact line per task, indented one level: `#70 Postcode API client · in progress · back-1` (ref, title, task status in words, assignee node). No glyph of the tracker, no labels, no actions except `⏎` = open the Feature pane on that task (§9.4) and `o` = open upstream.
- Never selectable with `x`, never matched by "Only ready" / "Only not started", never counted in children progress or roll-ups. Search can find them only with the filter "Include team tasks" (off).
- Tooltip on the fold: "Tasks the team of checkout-address wrote to GitHub. Manage them in the feature."

### 2.8 Board view: not drawn
Decision: **no board view** in this feature. Reasons: (1) GitHub upstream status is open / closed only unless a Projects v2 board is set, so columns would be two for most repos; (2) a board loses the hierarchy, which is the point of the browser; (3) the app-state question "what can I start now" is the Ready lane and the "Only ready" toggle; (4) the old Board's other use (team tasks) moves to the Feature pane. The `⋯` menu has no "Board" item. Revisit if users ask for a Projects v2 status view (F28).

---

## 3. Details panel (point 3)

Right side, scrolls on its own. Content for the focused node; updates on focus after 120 ms (no flicker on `j` repeat). First paint from the tree's `WorkNode`; body, children and history after `work_node(id)` ("Loading…" muted under the title).

```
shop-api › v2 › #12 Checkout v2                          (breadcrumb, clickable)
◯ #13  Address step                                      [running · checkout-address · executing]
Feature · open · checkout, ready · assignee gu · milestone v2
────────────────────────────────────────────
Body (plain text, ≤ 12 lines, Show more)
────────────────────────────────────────────
Sub-issues · 1 of 2 done
  ◉ #16 Postcode lookup                      merged
  ◯ #17 Address validation errors            —
  ▸ Team tasks · 4
Dependencies
  Blocking  #14 Payment step                 waiting · Checkout v2
  Blocked by  none
Linked pull requests
  #58 Postcode lookup · merged   #64 Address form · open
In Orchestrator
  12 Nov 09:40  Started in program Checkout v2 · feature checkout-address
  12 Nov 09:41  Labelled orch:running in GitHub
  12 Nov 11:02  Spec approved
Estimate
  Team run  ≈ 4.1–6.8 pp of the week   from 7 runs in shop-api
  Solo      ≈ 0.9–1.6 pp of the week   from 12 sessions across projects
  Room      12.0 pp left this week for shop-api
[Show feature g] [Open in GitHub ↗ o] [⋯ Hide from Ready ⌫ · Focus here z · Copy link]   (#13 runs: no start actions, §5.1)
```

| Part | Rules / copy |
|---|---|
| Breadcrumb | ancestors from the project, `›` separated, ≤ 3 then `…`; each opens that node in the tree |
| Title line | glyph, ref, title (wraps to 2 lines), app-state chip |
| Facts line | `<type> · <state> · <board status> · <labels> · assignee <a, b> · milestone v2` (GitHub; board status only with a board) / `<state> · <priority> · estimate 3 · cycle 14 · <labels> · <assignee>` (Linear) / `target 1 Dec 2026 · In progress` (Linear project). Only facts present |
| Body | P4.5 §3 rules (plain text, 12 lines, Show more, "Shortened…" line, load failure line). Milestones / Linear projects: their description; none → nothing |
| Children | "Sub-issues · 1 of 2 done" (GitHub) / "Issues · 2 of 6 done" (milestone, project) / "Sub-issues" (Linear). Up to 10 lines (glyph, ref, title, chip), then "+ 8 more · Show in tree". Team tasks fold line at the end. |
| Dependencies | Two rows "Blocking" / "Blocked by", each item a line with its chip; "none". Items in another repo / team: ref with owner `acme/api#4`, not openable in the tree ("Outside this tracker", opens upstream). |
| Linked PRs | `#58 Postcode lookup · merged` — opens in browser. Absent when none. |
| In Orchestrator (history) | newest last, ≤ 8 lines then "Show all". `what` copy per kind in F6. Empty: "Nothing yet. Start it here." |
| Estimate | P4.5 §3 copy for both modes. Parents with open children add "Program ≈ 10–17 pp of the week (3 features, per child)" computed by the plan; shown as "Run as program to see the plan's estimate" until a plan exists (F9) |
| Actions | Buttons only for the actions the node allows (§5.1); `⋯` holds Hide from Ready, Focus here, Copy link |
| Kinds | Milestone / Linear project / Linear milestone: breadcrumb, title, facts (due / target, status), description, children summary ("6 issues · 2 done · 1 running"), actions Run as program, Open in…; no Start solo / team run (F12). Project row: tracker line "GitHub acme/shop · Ready filter label ready", status, link "Tracker settings". |
| Multi-selection | when ≥ 2 selected: "3 selected under #12 Checkout v2" (as the selection bar, §5.4) + their list + Run selected as program / Clear |

---

## 4. Search and filters (point 4)

### 4.1 Search (`/`)
- Field in the toolbar, placeholder "Search work… (title, #12, ENG-42, text)". `/` focuses it; typing filters live (debounce 250 ms, `work_search`); `Esc` clears and returns to the tree (second `Esc` blurs).
- While a query is set, the tree is **replaced** by results: flat rows with the **breadcrumb** as a muted second line (row 44 px): `#17 Address validation errors` / `acme/shop › v2 › #12 Checkout v2 › #13 Address step`. Ref match ranks first (`#17`, `ENG-42` exact), then title, then body.
- Header line of results: "12 results for "address"" · "100+ results: refine the search" (cap 100) · "No results for "adress" in shop-api, mobile · Clear search". Filters apply to results; when filters hide some: "3 more hidden by filters · Show".
- `⏎` on a result reveals it in the tree (clears search, expands ancestors, focuses the row). `t` `a` `p` `o` act directly on the result.
- Searching cached data only; when the cache is older than 60 s a refresh runs in the background (header status shows "refreshing…").

### 4.2 Filters (`f`)
Popover under the button, two columns, applied live; footer "Reset" and "Done (Esc)".

| Filter | Control | Applies to | Copy |
|---|---|---|---|
| State | segmented: Open · Closed · All | GitHub, Linear (state type completed/canceled = closed) | "Hide closed" toggle in the toolbar mirrors "Open" |
| Only ready | checkbox | all | "Only items that match the Ready filter" |
| Only not started | checkbox | all | "Only items with nothing running, queued or merged" |
| Label | multi-select from the cache's labels | GitHub, Linear | "Has any of" |
| Assignee | multi-select, first item "Me" | GitHub, Linear | |
| Milestone / Cycle | GitHub milestone select; Linear cycle: Current · Next · Any | per tracker | |
| Type / Project | GitHub issue type multi-select; Linear project multi-select | per tracker | |
| Include team tasks | checkbox, off | all | "Show tasks written back by teams in search results" |

- Filters for a tracker kind not in scope are not shown. With several projects of both kinds, the popover shows both groups with the tracker name as group title.
- A filtered tree keeps **ancestors** of matching nodes (muted, not counted) so hierarchy stays readable. The header count reads "8 items match · 2 filters" and the button "Filters · 2".
- **Saved per scope** (F4): filters, toggles, grouping and expanded state are remembered per scope key (sorted project ids) across restarts. No named saved filters.
- No match: §11 "No match for filters".

---

## 5. Actions and dialogs (point 5)

### 5.1 Action availability
| Action | Key | Offered on | Disabled / hidden when (tooltip) |
|---|---|---|---|
| Start team run | `t` | issue / Linear issue / local item, open, app none or ready | project has no team → disabled "shop-api has no team yet"; running / queued / pr / solo → hidden, the chip says why; done (merged, item still open) → offered, the dialog adds a muted line "Ran before as checkout-address · merged 12 Nov"; closed → disabled "Closed in GitHub. Reopen it there to start it." |
| Start solo | `a` | same | same (minus the team rule) |
| Run as program | `p` | node with ≥ 1 **open** child that is not a team task (issue with sub-issues, milestone, Linear project / milestone; not on project rows) | leaf → hidden; node itself running / pr → hidden (its sub-issues are that run's scope); every child closed → disabled "No open sub-issues"; node already a program root → replaced by "Open program Checkout v2" (`p`) |
| Run selected as program | `p` with selection | ≥ 2 selected siblings | non-siblings → selection refused (§5.4) |
| Open in GitHub / Linear | `o` | every node with `url`; local: opens the file in nvim ("Open file") | |
| Hide from Ready | `⌫` | nodes with `ready: true` and not hidden | hidden ones: "Unhide" (`⌫`) |
| Focus here | `z` | node with children | |
| Show feature | `g` (or click the chip; the chip is not a Tab stop) | node with running / pr / done state | |

Every start opens a dialog; nothing starts on a key or click alone.

### 5.2 Start team run / Start solo (P4.5 §5 dialogs, additions)
- `work_prefill(id, mode)` replaces `intake_prefill`; dialogs unchanged otherwise (P4.5 §5.2 fields, estimate line, untrusted-text note, `stale` handling §5.3).
- **Origin chip** gains the breadcrumb: "From #13 Address step · in #12 Checkout v2 · GitHub ↗" (≤ 1 ancestor shown: the parent; tooltip full breadcrumb). Linear: "From ENG-101 Email sign-up · in Mobile onboarding › M1 · Sign-up · Linear ↗".
- **Brief** prefill lists the open sub-issues as the scope (core writes it, design shows it):
  ```
  # Address step (#13)
  In acme/shop › #12 Checkout v2. https://github.com/acme/shop/issues/13
  <body>
  ## Scope: sub-issues
  - [ ] #17 Address validation errors (Bug)
  - [x] #16 Postcode lookup (closed)
  ## Depends on
  - none
  ```
- Under the brief, one muted line when the item has open sub-issues: "2 sub-issues are part of this run's scope. To run them as separate features, use Run as program." with a link "Run as program instead" (opens the plan dialog for this node, closes this one after confirm if the brief was edited: "Discard your edits to the brief?").
- When the item is **blocked by** an open item: muted warning line above the buttons (fixture #14): "Blocked by #13 in GitHub (open). Starting now ignores that." Never disables.
- Write-back line (when on, §10.4), muted, above the buttons: "On start: #13 gets the label orch:running and a comment in GitHub." (acme/shop has no board; with a board: "#13 is moved to In progress…"; assign me on: "…assigned to you, labelled…").

### 5.3 Open in / Hide from Ready
- Open: browser for GitHub / Linear; nvim for local files. Toast only on failure: "Could not open the link: <reason>".
- Hide from Ready (`⌫`): as P4.5 §2.2, toast "Hidden #9 from Ready to start. It comes back if it changes in GitHub." with **Undo u** (5 s). The node stays in the tree with tag "Hidden" only when "Show hidden items" is on; otherwise its "ready" chip disappears (the node itself stays: hiding is from Ready, not from Work).

### 5.4 Multi-select
- `x` toggles selection on the focused row; `⇧j` / `⇧k` extend; `Esc` clears. Only **siblings** (same parent) can be selected together: `x` on a non-sibling shows a toast "Select items under the same parent to run them as one program." and does nothing. Team-task lines, project rows and closed items are not selectable (closed: toast "#20 is closed.").
- Selection bar (replaces the toolbar): "3 selected under #12 Checkout v2 · [Run selected as program p] [Clear Esc]". Count in words: "1 selected" disables the program button (tooltip "Select 2 or more, or use Run as program on the parent").

---

## 6. Program plan dialog (point 6)

Modal, 880 × up to 720 px (1512), 800 at 1280; opened by `p`; first paint "Planning Checkout v2…" (spinner / static) while `program_plan` runs.

### 6.1 Layout
```
Run as program · Checkout v2                                                          Esc
From #12 Checkout v2 · acme/shop · GitHub ↗
Split into  (•) One feature per sub-issue   ( ) Per sub-sub-issue   ( ) One feature for all
Order       (•) By dependencies   ( ) One after the other, in tree order
At once     [ 2 ▾ ] features running at most
──────────────────────────────────────────────────────────────────────────────────────
Nothing to fix · 1 note                                                  (issues block, §6.4: over_room)
──────────────────────────────────────────────────────────────────────────────────────
#  Feature                     From      Mode        Team          Estimate        Order
1  checkout-address            #13 +2    Team ▾      default-web ▾  ≈ 4–7 pp       first
   Address step
2  checkout-payment            #14 +1    Team ▾      default-web ▾  ≈ 5–8 pp       after 1 (⛓ #13 blocks #14)
   Payment step
3  checkout-summary            #15       Solo ▾      —              ≈ 1–2 pp       first
   Order summary
   [each row ⋯: Skip x · Merge into previous ⇧M · Edit brief e · Remove dependency]
──────────────────────────────────────────────────────────────────────────────────────
Graph  ① ──▶ ②     ③            (strip, only when there are edges)
──────────────────────────────────────────────────────────────────────────────────────
Total ≈ 10–17 pp of the week · 12 pp left this week for shop-api · over at the high end
Each feature still asks you to approve its spec and its PR.
Skipped: none · Left out: none                                         [Cancel] [Start program · 3 features ⌘⏎]
```

### 6.2 Controls and copy
| Control | Copy / rules |
|---|---|
| Split into | "One feature per sub-issue" (`child`, default, Q3) · "Per sub-sub-issue" (`grandchild`; disabled with tooltip "No sub-sub-issues" when none) · "One feature for all" (`single`). Milestone / Linear project root: "One feature per issue" · "Per sub-issue" · "One feature for all". Changing re-runs `program_plan`; edits made on rows are lost after confirm "Re-plan and lose your edits to 3 features?" |
| Order | "By dependencies" (default) · "One after the other, in tree order" (`sequential: true`; At once is forced to 1 and disabled, tooltip "One after the other runs one at a time") |
| At once | select 1–5, default 2 (Q5). Hint "Each running feature uses its own team." |
| Row: Feature | slug (mono, editable in the brief editor, `e`) + title; "From" = ref of the item + "+2" when its sub-issues are in its scope (tooltip lists them) |
| Row: Mode | select Team / Solo |
| Row: Team | select of project teams; "—" for solo; none → issue `no_team` |
| Row: Estimate | P4.5 rounding for the row's mode; `rough` prefix when basis default |
| Row: Order | "first" · "after 1" · "after 1, 2"; tooltip "#13 blocks #14 in GitHub" / "Sequential" / "Added by you" |
| Row ⋯ | **Skip** (`x`): row dims, Estimate struck from the total, Order of dependents recomputed (a dependent of a skipped feature waits for nothing from it; note "after 1 (skipped)"); **Merge into previous** (`⇧M`): the row's items join the row above, its brief appended; **Edit brief** (`e`): inline editor 10 lines under the row, Done with `⌘⏎`; **Remove dependency on 1**: cuts an edge (needed for cycles, §6.4) |
| Graph strip | 80 px, nodes numbered circles by order, arrows for edges, skipped dashed; read-only; hidden when no edges. Hover a node highlights its row |
| Totals | "Total ≈ 10–17 pp of the week" + room: "12 pp left this week for shop-api" + "· over at the high end" (high > room) / "· over this week's room" (low > room). Muted warning text, never blocks (P4.5 §2.1) |
| Gates line | "Each feature still asks you to approve its spec and its PR." (always) |
| Write-back line | when on: "As features start, GitHub items get the label orch:running (with a board: move to In progress); team tasks become sub-issues; merged PRs close their items, and #12 closes when all are done." |
| Left out line | "Closed sub-issues left out: #16" · "Already running, left out: #13 (checkout-address)" — muted |
| Start program | primary, accent fill, `⌘⏎`. Disabled while blocking issues remain (6.4), tooltip "Fix 1 thing first". Label with count: "Start program · 3 features" |

### 6.3 Graph vs list (decision)
**List with an "Order" column + a read-only graph strip.** The list is where every edit happens (keyboard, screen readers, 20+ rows); the strip only gives the shape at a glance. No editable graph canvas.

### 6.4 Plan issues and their fixes
Issues block: under the controls, `status.error` text for blocking, warning for advisory, each line with its fix action. Row with an issue gets a mark in its Order cell.

| `kind` | Blocking | Line | Fix (button on the line) |
|---|---|---|---|
| `cycle` | yes | "Order loop: checkout-payment and checkout-summary wait on each other (#14 ↔ #15 in GitHub)." | **Remove a dependency…** → popover listing the loop's edges "2 after 3" / "3 after 2" with radio, Apply. Or "Use tree order" (sets sequential). |
| `already_running` | no (item left out) | "#13 is already running as checkout-address. It is left out; features that depend on it wait for its merge." | **Show feature** |
| `closed` | no | "#16 is closed in GitHub. It is left out." | **Include anyway** (adds it back; rare) |
| `no_team` | yes | "checkout-payment needs a team: shop-api has no team." | **Make it solo** · **Create a team** (opens Team canvas) |
| `over_room` | no | high > room: "≈ 10–17 pp is over this week's room for shop-api at the high end (12 pp left). Features wait for room under the Governor."; low > room: "… is over this week's room …" (same split as §6.2 Totals) | none (advice) |

Header of the block: "1 thing to fix before starting" / "2 things to fix…" / "Nothing to fix · 2 notes" (advisory only) / absent when empty. On submit, `program_start` may return new issues (tree changed): dialog stays, block updates, toast-less.

### 6.5 States
Planning (spinner, controls disabled) · planned · re-planning (rows dimmed) · plan failed ("Could not plan Checkout v2: <message>" + Retry) · start failed (inline error, form kept) · stale (root deleted upstream or started as a program meanwhile: banner "#12 is already in program Checkout v2. Open program · Close" as P4.5 §5.3; a root **closed** meanwhile is not stale, S13: muted line "#12 was closed in GitHub. Its open sub-issues can still run.") · nothing to plan (every child closed or running: "Nothing left to run under #12: 2 closed, 1 running." primary **Close**).

---

## 7. Programs in Mission control, Program view, cards (point 7)

### 7.1 Lane and program row
- New lane **Programs** between Running and Waiting on (lane order: Needs you · Running · **Programs** · Waiting on · Queued · Ready to start · Done today · Idle). Not an attention lane (not in the badge). Header "Programs · 1 running". Lane absent with no program running or paused; done / cancelled programs move to Done today as one outcome line ("Program Checkout v2 done · 3 merged").
- **Program row** (52 px, two lines):
  ```
  ▾ ⧉ Checkout v2   from #12 · shop-api      1 of 3 merged · 1 running · 1 queued      [Open ⏎] ⋯
      ▰▰▱ progress (merged / not skipped)                                     ≈ 10–17 pp week
  ```
  - Counts from `now.programs[].counts`, only non-zero, order merged, running, pr ("1 PR open"), waiting, queued, failed ("1 failed" error text), skipped.
  - Paused: "paused" tag (`status.paused`) after the title; counts unchanged.
- **Sub-rows** (28 px each, one per feature, indented, expanded by default when ≤ 5 features, else collapsed with "<n> features ▸", e.g. "6 features ▸"):
  | Feature state | Sub-row |
  |---|---|
  | merged | `✓ checkout-address · merged #64` (muted) |
  | running | `● checkout-payment · executing · default-web` → `⏎` opens the feature; the same run also appears as its normal row in Running/Needs you with origin "from #14 · Checkout v2" |
  | pr | `◐ checkout-payment · PR #66 open` |
  | waiting (deps) | `○ checkout-payment · waiting for checkout-address to merge` |
  | queued (Governor) | `○ checkout-summary · queued · live cap 2/2` (also a Queue row in Queued, P4.2 §1.5, origin "Checkout v2") |
  | skipped | `– checkout-x · skipped` (muted) |
  | failed | `✕ checkout-x · failed · Open` (error text) |
  | flagged | tail `· closed in GitHub` / `· new dependency` (warning text) |
- Origin chip on run / queue rows (P4.5 §6) gains the program: "from #14 · Checkout v2".

### 7.2 Program view (pane, opened by `⏎` on the row, the chip, the palette "Program: Checkout v2")
```
Checkout v2 · program                                 running · 1 of 3 merged     [Pause p] [⋯ Cancel program ⇧K]
From #12 Checkout v2 · acme/shop ↗ · started 12 Nov 09:40 · 2 at once · by dependencies
─ graph strip (as 6.3, live states) ─
#  Feature             From  Mode  State                                   Estimate   Actual
1  checkout-address    #13   Team  merged #64 · 12 Nov 15:10              ≈ 4–7      5.2 pp
2  checkout-payment    #14   Team  running · executing                    ≈ 5–8      2.1 pp so far
3  checkout-summary    #15   Solo  queued · live cap 2/2                  ≈ 1–2      —
   row ⋯: Open feature ⏎ · Skip x · Open item o
New in GitHub: #19 Gift cards (sub-issue of #12, added 13 Nov)          [Add to program a] [Ignore]
Settings: At once [2 ▾]
```
| Action | Copy / rules |
|---|---|
| Pause / Resume (`p`) | Pause: no new feature starts; running ones continue. Confirm-less, toast "Checkout v2 paused. Running features continue." / "Checkout v2 resumed." |
| Skip feature (`x`) | only planned / waiting / queued. Confirm "Skip checkout-summary? Features after it no longer wait for it." [Skip] |
| Add new upstream item (`a`) | opens a small plan row editor (mode, team, after which feature) prefilled by the core; [Add] |
| At once | select, applies to the next start |
| Cancel program (`⇧K`) | Confirm "Cancel Checkout v2? Running features keep running as normal features; waiting ones are dropped." [Cancel program] [Keep] |
| Done | header "done · 3 merged · 13 Nov 18:20"; read-only; outcome line "Program Checkout v2 done" in chat |

### 7.3 Cards (chat column, P4.3 / P4.4 card anatomy)
| Card | Level | Title / body | Actions |
|---|---|---|---|
| New child upstream | 3 (needs you, not urgent) | "New sub-issue in Checkout v2" / "#19 Gift cards was added under #12 in GitHub." | **Add to program ⏎** (opens the row editor in Program view) · **Ignore r** |
| Child closed upstream | 3 | "#15 was closed in GitHub" / "checkout-summary is in Checkout v2 and is waiting. It is not stopped." | **Skip it ⏎** · **Keep r** |
| Dependency added | 4 (info) | "New order in Checkout v2" / "#15 is now blocked by #14 in GitHub: checkout-summary waits for checkout-payment." | **Open program ⏎** |
| Program done | 4 | "Checkout v2 is done" / "3 merged, 0 skipped · ≈ 9.8 pp used of 10–17 planned · #12 closed in GitHub." | **Open program ⏎** |
| Program failed feature | 3 | "checkout-payment failed in Checkout v2" / reason | **Open feature ⏎** · **Skip it x** |

Collapsed lines: "added #19 to Checkout v2 · by you", "ignored #19", "skipped checkout-summary", "program Checkout v2 done".

### 7.4 Spec and PR gates
Spec / PR cards of program features get a tag line "Checkout v2 · 2 of 3" under the title.

### 7.5 Batched spec approval: not recommended
With max 2 at once and dependency order, at most 2 specs are pending together; a batch card would hide each spec's content behind a list and invite approving unread specs. Instead: (1) program tag (7.4); (2) after approving a program spec, the next pending spec of the same program gets focus ("Next: checkout-summary's spec · ⏎"). Revisit if programs with At once ≥ 4 are common (F29).

---

## 8. Ready to start lane (point 8; amends P4.5 §1–§4)

- Row line 2 gains the **breadcrumb** (from `breadcrumb`, ≤ 3, root first, parent only when narrow): "in #12 Checkout v2 › #13 Address step" (`text.muted`, ellipsis from the left). Order on line 2: warning · breadcrumb · project · labels. At 1280: breadcrumb shows the parent only: "in #13 Address step".
- **Browse all**: header button, left of Refresh: "Browse all" (`w` on the lane header or with no Ready row focused: the lane's scope, i.e. the filter's projects; with a Ready row focused `w` reveals that item in Work, §12.2). With one project shown: tooltip "Open Work for shop-api".
- **Program-queued items**: items whose app state is queued / waiting in a program **leave** the Ready lane (they are in Programs). Items that are a program root also leave. No "queued" variant of the Ready row.
- Ready rows of a parent with open children gain a third action in `⋯`: "Run as program" (`p`).
- No-source line (P4.5 §4.1) copy: "shop-api has no tracker · Plan with the Overwatch · Set up tracker" (`e` opens Project settings › Tracker). Error lines and Fix targets point to Tracker.

---

## 9. Feature pane (point 9)

### 9.1 Source header
Top of the pane, above the docs, one block (no card border), 2 lines:
```
◯ #13 Address step · GitHub ↗                                 [Show in Work w]
in acme/shop › v2 › #12 Checkout v2 · program Checkout v2 (1 of 3)   · Write-back on
```
- Linear: "◐ ENG-110 Welcome tour · Linear ↗" / "in Mobile onboarding › M2 · First run".
- No source (started by hand or a brief): line "No tracker item · started by you" / "From the Overwatch's brief: Partial refunds ↗". No Show in Work.
- Write-back state: "Write-back on" / "Write-back off" (muted), and failures (§11): "Write-back failed: could not move #13 to In progress · Retry".
- Item closed upstream while running: tail "· closed in GitHub" (warning text), tooltip "The run continues. Stop it from the feature if needed."

### 9.2 Tasks section (the moved board)
- Heading "Tasks" + right "GitHub sub-issues of #13 · 1 of 4 done" (write-back on) / "Local · 1 of 4 done" (off or local).
- Columns are the Board's five, in its order (Backlog · Ready · In progress · Review · Done; today's Tasks section has no Backlog column and would lose backlog tasks once the Board is gone), so `⇧H` / `⇧L` follow the status flow. The Board's moves on a **focused task**: `⏎` menu, `a` assign (node picker), `m` move to (status), `1–5` column, `e` edit (title / body dialog, writes back), `o` open upstream (`#70` in GitHub), `n` new task (dialog), `⇧H` `⇧L` move left / right. `j k` within a column, `h l` across columns — **only when the Tasks section has focus** (`b` focuses it; `Esc` returns to the pane).
- Task card: `T-4 · #70` (local id and upstream ref when written back), title, model chip, assignee chip, "blocked by T-2" mark (existing).
- Sync state per task: tiny mark "not synced" (`text.muted`) with tooltip when write-back is pending; "conflict" (warning) when a sync conflict exists, menu item "Resolve conflict…" (existing conflict flow).

### 9.3 Pane keys change
`b` = focus Tasks (was Board). `w` = Show source in Work. Others unchanged (§12).

### 9.4 Landing from links
A sync-conflict outcome, a `T-n` waiting chip or a chat task link: opens (or focuses) the Feature pane of the task's run, scrolls Tasks into view, focuses the task, flashes it once (200 ms, not under reduced motion), and for a conflict opens its menu at "Resolve conflict…". Task whose run is gone: toast "T-12 belongs to checkout (deleted). Open #70 in GitHub?" with **Open o**.

---

## 10. Project settings › Tracker (point 10; supersedes P4.5 §7 and the Task backend section)

Sections: General · Files · **Tracker** · Allocation · Notion · MCP. Heading "Tracker", subtitle "Where this project's work lives. Work, Ready to start and team tasks all read it."

### 10.1 Layout
1. Migration notice (§10.6), when present.
2. **Kind**: segmented **Local · GitHub · Linear**.
3. Fields per kind (10.2).
4. **Ready filter** (10.3).
5. **Write back** (10.4).
6. **Refresh**: select "Every 5 min · 10 min · 30 min · 1 h", hint "Work also refreshes when you open it and on r." (`refresh_secs`).
7. **Preview** (10.5).
8. Footer: Save `⌘⏎` · Revert; unsaved marker (P4.5 §7.1).

### 10.2 Fields per kind
| Kind | Field | Control / copy |
|---|---|---|
| Local | Backlog folder | text, default `.orchestrator/backlog/*.md`; empty = "No backlog files: only tasks you create". Live match line (P4.5 §7.2). Hint "Local has no hierarchy: Work shows a flat list." |
| GitHub | Repository | as P4.5 §7.2 ("Defaults to this project's repository.") |
| GitHub | Projects v2 board (optional) | select from the F17 `boards` list (P4.5 F19) "acme · #4 Roadmap" / "None"; hint "Shows its Status in Work and lets write-back move items on it." `gh_scope` inline (10.7) |
| Linear | Teams | multi-select chips from Linear teams; required "Pick at least one team." |
| Linear | Projects | radio "Every project of these teams" (default) · "Only these projects" + multi-select |
| Linear | Team task states (fold "Advanced") | the existing mapping (each Linear state → one of the five columns Backlog / Ready / In progress / Review / Done, defaults per state type) moved here unchanged from Task backend; hint "Where team tasks go when they move in the feature." |

### 10.3 Ready filter
Heading "Ready to start shows", hint "Mission control lists these items. Work shows everything."
- GitHub: Labels (tags, default `ready` if it exists, else empty = "Nothing: the lane stays empty until you pick something"), Milestone (select, Any), Board status (select from the board's statuses, only with a board), Issue type (select, Any). All combined with "and": summary line "Open issues labelled ready, in milestone v2".
- Linear: State types (checkboxes Backlog · Unstarted, default Unstarted), Label (Any), Project (Any), Cycle (Current · Next · Any, default Current). Summary "Unstarted issues in the current cycle".
- Local: "Every backlog file not marked done."

### 10.4 Write back (plain words)
Block with a switch **"Update GitHub as work runs"** / **"Update Linear as work runs"** (default on, Q1); not shown for Local.

Under it, what it does, as a list (muted, updates with the options):
- "When you start an item, it moves to **In progress** and gets a comment "Started in Orchestrator"." (GitHub without board: "…gets the label `orch:running`…")
- "Team tasks are created as sub-issues of the item, labelled `orch`."
- "Pull requests say `Closes #13`, so merging closes the item." (Linear: "Pull requests link the issue; merging moves it to Done.")
- "A program's parent closes when all its items are done."
- "Nothing else is changed, and only items of acme/shop."

Options: checkbox "Assign the item to me when I start it" (off) · GitHub without board: field "Running label" (default `orch:running`, hint "Added on start, removed on merge").
Off copy: "Read-only: GitHub stays as it is. Team tasks stay in the app."

### 10.5 Preview
`tracker_preview`: button "Preview"; result "**14 items**, 3 at the top · **4 ready** in acme/shop" + up to 5 sample rows with breadcrumb + "Changed since this preview" stale state (P4.5 §7.3 rules).

### 10.6 Migration notice
Shown once per project after the update until dismissed (or resolved for a mismatch), info style (neutral `surface.raised`, info glyph, no error colour; no new token) except the mismatch which uses warning.
| Case | Copy | Actions |
|---|---|---|
| Same repo / team | "Task backend and Intake are now one setting: the tracker. acme/shop is used for both, and your Ready filter (label ready) is kept." | **Got it** |
| Different repo, same kind | "Task backend used acme/shop and Intake used acme/web. The tracker keeps acme/shop; the Intake label ready is now its Ready filter. acme/web is no longer read." | **Got it** · **Use acme/web instead** |
| Mismatch kind | "Intake read GitHub acme/shop but tasks were in Linear ENG. Pick the tracker for shop-api:" radio **GitHub acme/shop** · **Linear ENG** + "Until you pick, Work shows Linear ENG." | **Use this tracker** (saves) |
| Local + repo docs | "Intake's repo docs folder is now the Local tracker's backlog." | **Got it** |

### 10.7 Errors (Preview, Work header, Ready lane line)
P4.5 §4.2 table stands with Fix targets → Tracker. Additions:
| Code | Line | Fix |
|---|---|---|
| `linear_complexity` | "Linear said the request was too large. Work shows what is cached." | **Fix** (`settings`): "Limit the tracker to fewer projects" (opens Projects field). Also in Work: "Some items may be missing." |
| `gh_sub_issues` | "GitHub didn't return sub-issues for acme/shop. Work shows issues without hierarchy." | none; **Retry** |
| `gh_scope` (board) | "GitHub needs the read:project scope to read the board." | **Copy command** `gh auth refresh -s read:project` |
| write-back `gh_write` (F27; Linear `linear_write`) | "GitHub refused the change: your login can't write to acme/shop." | **Turn off write-back** (in settings) |
| field `teams` | "Pick at least one team." | |

---

## 11. States matrix (point 11)

| # | State | Where | Copy / form |
|---|---|---|---|
| S1 | **First load** (no cache) | Work | header "Loading acme/shop…"; tree: 6 skeleton rows (static under reduced motion); per project row "Reading GitHub…" / "Reading Linear…" |
| S2 | **Refreshing** (cache shown) | Work header, Ready lane | "refreshing…" (spinner 12 px); rows stay; Refresh disabled "Refreshing…" |
| S3 | **Offline / source unreachable, cache** | Work header + banner line | "Offline · updated 2 h ago" (header, muted); line under toolbar "Showing what was cached at 13:02. Starting still works; write-back waits." ; Refresh → retries |
| S4 | **Source error** (per code §10.7, P4.5 §4.2) | Work: project row tail + line under it | `shop-api · GitHub: not logged in · Fix`; cache rows kept below with header time |
| S5 | **Rate limited** | Work, lane | "GitHub rate limit reached · retry at 14:20"; Refresh disabled till then, tooltip "Retry at 14:20" |
| S6 | **Linear complexity** | Work | §10.7 + rows partially loaded; a parent that failed to load: child line "Could not load sub-issues: request too large · Retry" |
| S7 | **Empty tracker** | Work | "No open issues in acme/shop. Create them in GitHub ↗; Work picks them up at the next refresh (r)." Linear: "No issues in ENG." Local: "No backlog files in .orchestrator/backlog/. Add a .md file there." |
| S8 | **No match for filters** | Work | "Nothing matches 3 filters in shop-api · Reset filters" (`⏎`) |
| S9 | **No match for search** | Work | §4.1 |
| S10 | **Huge tree** (500+ roots) | Work | roots paged 100 at a time: last line "Showing 100 of 512 · Load 100 more" (`⏎`); header "512 at the top". Virtualised list (no visible change). Search recommended in the line: "or search with /" |
| S11 | **Parent with 100 children** | Work | first 50 shown, "+ 50 more sub-issues · Load" line; GitHub limit note at 100: "GitHub shows at most 100 sub-issues per issue." |
| S12 | **Deep tree** | Work | §2.4 with the deep fixture, Focus here board |
| S13 | **Closed parent with open children** | Work | parent glyph closed + children open shown normally; the parent row's tail "closed · 2 open sub-issues" (muted); Run as program still offered on it ("2 open sub-issues") |
| S14 | **Item deleted upstream while running** | Work, Feature header, MC run row | node stays (from cache) with tag "Deleted in GitHub" (warning text), glyph dimmed; actions only Show feature; Feature header "#13 was deleted in GitHub. The run continues." ; origin chip unchanged, tooltip "Deleted in GitHub" |
| S15 | **Write-back failed** | Work row, Feature header, toast once | row tail "write-back failed" (warning); details history line "13 Nov 09:41 Could not label #13 orch:running: <reason> · Retry"; Feature header line with **Retry**; one toast per failure kind per feature: "Could not update #13 in GitHub. The run continues. Retry from the feature." Repeated failures (3+) → card level 3 "Write-back is failing for shop-api" · **Open settings** · **Turn off write-back** |
| S16 | **Loading children** | Work | "Loading…" child line under the expanded node; failure "Could not load: <reason> · Retry" |
| S17 | **Hidden items shown** | Work | hidden nodes are always in the tree (hiding is from Ready); with "Show hidden items" on they get tag "Hidden", 60 % text, action Unhide |
| S18 | **Scope with Local + GitHub + Linear** | Work | three project rows, each with its own status |
| S19 | **gh_scope** | Settings preview, Work line | §10.7 with Copy command |
| S20 | **Program states** | MC, Program view | running, paused, flagged child, failed feature, done, cancelled (7.1–7.3) |
| S21 | **Plan dialog states** | dialog | 6.5 + every issue kind of 6.4 |
| S22 | **Migration notices** | settings | four cases 10.6 |
| S23 | **Selection** | Work | 1, 3 selected; non-sibling refusal toast |

---

## 12. Keys (point 12; page 05)

### 12.1 Work browser (pane-local, unmodified single keys, P4.2 §3.1 conventions)
| Key | Action | On |
|---|---|---|
| `j` `k` / ↓ ↑ | Move | tree, results |
| `h` / ← | Collapse; on a collapsed or leaf row: go to parent | row |
| `l` / → | Expand; on an expanded row: first child | row |
| `⏎` | Open details (and focus them when already shown); on lines: their action (Load more, Retry, Fix, Reset); on a result: reveal in tree | row · line |
| `space` | Toggle details panel | pane |
| `/` | Search | pane |
| `f` | Filters | pane |
| `x` | Select / unselect (siblings only) | row |
| `⇧J` `⇧K` | Extend selection down / up | row |
| `t` | Start team run (dialog) | startable item |
| `a` | Start solo (dialog) | startable item |
| `p` | Run as program (dialog); with a selection: run selected; on a program root: open program | node with open children |
| `o` | Open in GitHub / Linear (local: file in nvim) | row |
| `⌫` | Hide from Ready / Unhide | ready row |
| `z` / `⇧Z` | Focus here / up one level | node with children / focus mode |
| `g` | Show feature (of the app-state chip) | row with a feature |
| `r` | Refresh the scope | pane |
| `u` | Undo the last Hide (5 s) | pane |
| `Esc` | Clear search → clear selection → close details → leave focus mode (in this order) | — |

### 12.2 Mission control additions
| Key | Action | On |
|---|---|---|
| `w` | Open Work (filter's projects); on a Ready row: Work revealed at that item | pane |
| `p` | Run as program | Ready row with open children (the agent `p` = pause applies to agent rows only) |
| `⏎` | Open Program view | program row |
| `space` | Expand / collapse sub-rows | program row |
| `p` | Pause / resume program | program row |
| `⇧K` | Cancel program (confirms) | program row |
| `o` | Open the source item | program row |

### 12.3 Program plan dialog
`j k` rows · `x` skip / unskip · `m` toggle mode team/solo · `⇧M` merge into previous · `e` edit brief · `⌘⏎` Start program · `Esc` cancel (asks when edited: "Discard this plan?"). In a text field single keys type.

### 12.4 Program view
`j k` rows · `⏎` open feature · `x` skip feature · `a` add the new upstream item · `o` open item · `p` pause / resume · `⇧K` cancel program · `Esc` back.

### 12.5 Feature pane
`b` focus Tasks (was Board) · `w` show source in Work · Tasks focused: `j k h l`, `⏎` menu, `a` assign, `m` move to, `1–5` column, `e` edit, `n` new task, `o` open upstream, `⇧H ⇧L` move card, `Esc` back to the pane. **Changes**: the pane's own `e` (open doc in nvim) and `m` (message) and `p` (pause team) apply only when Tasks is **not** focused.

### 12.6 Global
Leader `⌘; b` = Work browser (context project or all). No ⌘ chord added.

### 12.7 Conflict check
| Against | Result |
|---|---|
| **Mission control map** (`j k h l ⏎ space i p s r ⇧K d n f o ⌫ Esc` + P4.5 `t a x e ⇧R ⇧H u`) | `w` new, unbound → OK. `p` on a **Ready row** = Run as program; `p` on agent / run rows stays Pause: different row kinds, the hint row says which (same split as `⌫`). Program row reuses `p` (pause) and `⇧K` (cancel/kill) with the same meaning family. `o` on program row = open source, consistent. |
| **Contract §5 proposal** | `s` for solo **dropped** → `a` (P4.5 §10.3: `s` is Suspend). `⏎` = details as proposed; start keys stay letters. `x` = select (proposal) — differs from Ready rows where `x` hides; Work uses `⌫` for hide (both panes accept `⌫` for hide, so `⌫` is the portable key). Contract §5 to update (F20). |
| **Work vs Mission control `r`** | `r` = Refresh in Work (no agents there); Resume in Mission control; the Ready lane keeps `⇧R`. Different panes. |
| **Feature pane** | `a` (assign), `m` (move), `e` (edit), `n` (new) inside Tasks shadow the pane's `e` (doc) / `m` (message) only while Tasks has focus; `Esc` leaves. `b` relabelled. `w` new. |
| **Board map** | removed with the Board (`g` remote → `o`; `r` sync → Refresh in Work; `⇧O` off-hours dropped: a launch option, not a task action). |
| **Leader** (`j k t ] [ e w p o g`) | `b` free → Work. |
| **Global ⌘ chords** (⌘K P B I N T W J L , / ⇧T ⇧I) | none added. ⌘B stays sidebar. |
| **Chat / Inbox** (`a r ⏎ o x u e t`) | separate focus target; program cards use `⏎` / `r` / `x` as cards do. |
| **Text fields** | search field and dialog fields swallow single keys; `Esc` leaves the field first. |

Result: **no binding collision**; meaning splits by row kind: `p` (Run as program on Ready rows, Pause program on program rows), `space` (Details elsewhere, expand / collapse sub-rows on a program row: its sub-rows are its details), same style as the accepted `⌫` split; one contract change (`s` → `a`). Page 05 gets 12.1–12.6 and the note "`s` = Suspend; solo is `a` everywhere".

Palette entries: "Open Work (all projects)", "Work: shop-api", "Find work item…", "Start team run: #13 Address step", "Run as program: #12 Checkout v2", "Program: Checkout v2", "Pause program Checkout v2", "Refresh Work".

---

## 13. For ui: what to draw (hand-off)

Every board at **1512 and 1280** (spec §5), including the rows below that name no width; 1280 applies the §2.2 / §2.5 / §8 narrow rules.

| # | Board | Page | Notes |
|---|---|---|---|
| 1 | Mission control with Work button, Programs lane (Checkout v2 running), Ready lane with breadcrumbs and Browse all — 1512 and 1280 | 04.2 | §1.2, §7.1, §8 |
| 2 | Work · GitHub fixture, details on #13 — 1512, 1280 | 04.8 Work (new) | §2, §3 |
| 3 | Work · Linear fixture, details on ENG-101; and multi-project scope (shop-api, mobile, blog) | 04.8 | §2.1 project rows |
| 4 | Tree row anatomy + every app-state chip + kind glyphs + roll-up + team tasks fold expanded | 02 | components Tree row, App-state chip, Kind glyph |
| 5 | Search results, filters popover, no match | 04.8 | §4 |
| 6 | Deep fixture: L5/L6 tags, Focus here breadcrumb | 04.8 | §2.4 |
| 7 | Multi-select bar, refusal toast | 04.8 | §5.4 |
| 8 | States S1–S19 (one board per row of §11, grouped 4 per frame) | 04.8 | §11 |
| 9 | Launch / New-session prefilled with breadcrumb, sub-issues brief, blocked line, write-back line | 04.4 | §5.2 |
| 10 | Program plan dialog: planned (fixture), each issue kind, re-plan confirm, nothing to plan, sequential | 04.4 | §6 |
| 11 | Program view: running, paused, new upstream item, done | 04.4 | §7.2 |
| 12 | Feature pane: source header (GitHub, Linear, none, write-back failed, deleted upstream) + Tasks section focused with menu | 04.4 | §9 |
| 13 | Board boards stickered "Removed in improve-intake" | 04.4 | §1.3 |
| 14 | Program cards (new child, closed upstream, dependency added, done, failed) + collapsed lines + spec card program tag | 04.3 | §7.3–7.4 |
| 15 | Project settings › Tracker: Local, GitHub (with board), Linear (teams, projects, advanced); Ready filter; write-back on/off; preview; four migration notices; errors | 04.7 | §10 |
| 16 | Program row component + sub-row states | 02 | §7.1 |
| 17 | Keys tables | 05 | §12 |

---

## 14. Findings for lead

No change made to spec or contract. ui draws the proposal unless lead decides otherwise.

| # | Finding | Proposal |
|---|---|---|
| F1 | **AppState lacks `ready` and `closed upstream`**, which spec §3.3 lists as app states. | UI derives them: `ready` from `WorkNode.ready` when `app.state = none`; closed from `upstream`. No contract change; say so in contract §1.2. |
| F2 | **`rollup` scope undefined** (direct children or descendants; team tasks in or out). | Descendants, excluding `team_task` nodes and hidden ones; add `waiting` (queued with `waiting_on`) separate from `queued`. |
| F3 | **`filter` of `work_tree` / `work_search` has no shape.** | `WorkFilter = { state: "open"|"closed"|"all", only_ready, only_not_started, labels[], assignees[] ("@me"), milestone?, cycle?: "current"|"next", issue_types[], linear_projects[], include_team_tasks, show_hidden }`. |
| F4 | **"Saved per scope" (brief §2.4) has no storage.** | UI preference via `settings_set` key `work.view.<sorted project ids>` (filters, toggles, expanded ids ≤ 500, focused id). No new command. |
| F5 | **`work_search` breadcrumb shape** is "WorkNode + breadcrumb" without a type. | Same as `now.intake` (`[{ref, title}]`, root first, ≤ 3) plus `id` per crumb so the UI can reveal it. |
| F6 | **`history[].what` is free text.** UI needs stable kinds for copy and icons. | Enum `started | program_queued | spec_approved | pr_opened | merged | skipped | writeback_done | writeback_failed | returned` + `detail?`. Copy: "Started in program Checkout v2 · feature checkout-address", "Labelled orch:running in GitHub" (with a board: "Moved to In progress in GitHub"), "PR #64 opened", "Merged · PR #64", "Could not label #13 orch:running: <reason>". |
| F7 | **New upstream children of a running program** have no field; Program view needs to list them. | `Program.new_items: [WorkNode id]` and `program_update` patch `{ ignore_item }`. |
| F8 | **PlanIssue refers to features only**, but `closed` and `already_running` concern items left out of every feature. | Add `items: [WorkNode id]` and, for `already_running`, `feature: slug`; `cycle` adds `edges: [[from, to]]`. |
| F9 | **Program estimate has no room.** The dialog compares the total with the week's room. | `Program.estimate` gains `room_pp`, `room_scope`, `over_room: "none"|"high"|"low"` (P4.5 F6 shape). |
| F10 | **Edges have no origin** (tracker dependency, sequential, manual), and spec's planned feature `base` is missing. | `ProgramFeature.depends_on` stays; add `edges: [{from, to, source: "tracker"|"sequential"|"user", items?: [id, id]}]` on Program; add `base`. Removing an edge in the draft = dropping it from `edges` with `source: "user"` cut list `cut: [[from,to]]` so a re-plan keeps the cut. |
| F11 | **`program_plan` re-plan loses edits.** | Accept: confirm in the UI (§6.2). No change. |
| F12 | **Linear / GitHub milestone and project nodes:** Start solo / team run on them is not defined. | Not offered (only Run as program); a milestone as one feature = grouping "single". Confirm. |
| F13 | **Item deleted upstream** has no flag. | `upstream.deleted: true` kept in cache while any app link exists; otherwise the node is dropped. |
| F14 | **Write-back failures have no state nor retry.** Event only. | `WorkNode.writeback?: { status: "ok"|"pending"|"failed", what, at_ms, message? }`; command `writeback_retry(node)`; decision kind `writeback_failing` (level 3) after 3 failures in a project. |
| F15 | **One `SourceStatus` per `work_tree` call** but scope has several projects; no retry time. | `status: { [project]: SourceStatus }`; `SourceError.retry_at_ms?` (P4.5 F2); `offline: bool` when the cache is served because the network failed. |
| F16 | **`migration_notice` has no shape.** | `{ kind: "merged"|"different_repo"|"mismatch"|"backlog", from: {task_backend, intake_source}, kept, alternative?, dismissed: bool }` + `tracker_notice_dismiss(project)`. |
| F17 | **No options command for the tracker** (labels, milestones, issue types, boards and statuses, Linear teams / projects / cycles). | `tracker_options(project, partial_tracker)` = P4.5 `intake_options` extended with `issue_types`, `boards: [{owner, number, title, statuses[]}]` (P4.5 F19), Linear `teams`, `projects`. |
| F18 | **`more` paging per parent**: `work_tree` returns `more: n` once; children paging of a parent with 100 sub-issues needs a cursor. | `work_tree(scope, parent, filter, after?)` returns `cursor?`; UI "Load 50 more". |
| F19 | **GitHub milestone grouping** is optional in spec but not in the model. | View option `group_by_milestone` (default on when the repo has open milestones) passed to `work_tree`; off → milestone is only a filter. |
| F20 | **Contract §5 keys**: `s` solo collides with Suspend; `⌫` vs `x` hide differs from Ready rows. | §12: solo `a`; `x` select; `⌫` hide; `w` in Mission control; leader `⌘; b`. Update contract §5. |
| F21 | **Programs lane** not named in contract §2 order. | Lane order `needs_you, running, programs, waiting, queued, ready, done`; program done → outcome in Done today (`OutcomeAttention` kind `program`). |
| F22 | **Program cards** have no decision kinds. | Kinds `program_new_item`, `program_item_closed`, `program_dependency_added`, `program_done`, `program_feature_failed` with actions `add|ignore`, `skip|keep`, `open`. |
| F23 | **Feature pane source** not in the feature/run model. | `run.source?: { node, ref, title, url, breadcrumb, program? }`. |
| F24 | **Task ↔ feature mapping for links** (`T-n`, sync conflict) must reach the run. | Task records keep `run`; UI opens `openFeature(run, {task})`. Back: confirm every task has `run` (team tasks always do). |
| F25 | **Linear team-task state mapping** (Board column mapping) has no home in TrackerSettings. | `TrackerSettings.task_states?: { "<Linear state id>": "backlog"|"ready"|"in_progress"|"review"|"done" }` (Linear only): today's saved state → column mapping, moved as is (it reads tasks in and picks the state to write); UI under Tracker › Advanced. |
| F26 | **`gh_sub_issues` fallback**: is the tree flat or the call failing? | Flat tree with status `ok` + `warning: "gh_sub_issues"`; not an error (items still usable). |
| F27 | **Write-back permission error** has no code. | `gh_write` / `linear_write` codes on `writeback.failed`; copy in §10.7. |
| F28 | **Board view** (brief §2.2 optional) dropped. | Lead to confirm; revisit with Projects v2 status view later. |
| F29 | **Batched spec approval** not recommended (§7.5). | Lead to confirm; adds `program` tag on spec/PR decisions (`decision.program?: {id, title, index, total}`). |
| F30 | **Selection of non-siblings** for a program (spec says siblings). | Keep siblings only (§5.4); cross-parent programs out of scope. |
| F31 | **`children` counts include team tasks.** GitHub's `subIssuesSummary` (and Linear `children`) count the `orch` sub-issues written back (#13: 2 sub-issues + 4 team tasks = 6), but progress, Run as program and roll-ups exclude them (§2.2, §2.6, §5.1). | Core computes `children { total, open, done }` without `team_task` nodes and adds `team_tasks: n` for the fold. |
| F32 | **`now.programs` is too thin for the program row** (§7.1): no source ref, estimate, phase, team, PR number, merged PR. | Add `root_ref` ("#12"), `estimate {low_pp, high_pp}`, and per feature `title`, `mode`, `team`, `phase?`, `pr?: {number, url}`. |
| F33 | **`linked_prs` have no title** (details show "#58 Postcode lookup · merged"). | Add `title` to `linked_prs[]`. |
| F34 | **`AppState` solo has only `agent`**; the chip shows the session's name ("solo · fix-feed"). | Add `name` to the `solo` state. |
| F35 | **Write-back target with a board / in Linear is unnamed.** "Moved to In progress" needs a board Status option (names vary: "In Progress", "Doing") and a Linear `started` state (a team may have several). | `write_back.board_status?: "<option>"` (default: first option named like "In progress") and `write_back.linear_state?: "<state id>"` (default: first `started` state); picker in §10.4 under the switch. |
