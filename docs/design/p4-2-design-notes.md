# P4.2 design notes: copy, states and keys for Mission control

UX pass for P4.2 (Mission control), 2026-10-29, by `ux` for `ui` (then `front`). Sources: [spec](../../.orchestrator/features/p-4-2-mission-control/spec.md), [UX brief](../../.orchestrator/features/p-4-2-mission-control/ux-brief.md), [decisions](../../.orchestrator/features/p-4-2-mission-control/decisions.md), [API contract](../../.orchestrator/features/p-4-2-mission-control/api-contract.md), SPEC §9.1–9.7 and J12, and [P4.1 notes](p4-design-notes.md) (called **P4.1 §n** below; its copy rules and strings are the base and are not repeated).

This file is the "P4.2" part of the P4 notes; it passed ~200 lines, so it is a file of its own. Copy rules: sentence case, no exclamation marks, name the agent and the project, pp for usage, local 24 h times, and **every number says what it is** (principle 11).

---

## 0. Findings for lead

| # | Finding | Proposal |
|---|---|---|
| F1 | **Lane counts and rows disagree.** Spec §3.2 says lane counts are the core's per-project counts, and those count agents. The rows are runs and solo sessions, and a run sits in one lane only (D4). Fixture: the core says Running = 2 agents (lead-1, fix-feed) but the Running lane has 1 row (the checkout run is in Needs you). "Running · 2" over one row reads as a bug, and "Needs you · 1" over a row with a run and a blocked chip reads as incomplete. | The lane header counts **rows**, with a unit: "Needs you · 1 run", "Running · 2 sessions", "Needs you · 1 run · 1 session" when mixed. Queued counts agents (one row each): "Queued · 2 agents". Done today counts outcomes: "Done today · 4 outcomes". The core's per-project agent counts stay where agents are the unit: footer, status bar, rail tooltips, project-filter popover. Spec §3.2 "never recounted from rows" is dropped for the lane header; the filter then needs no summing. If lead keeps the core's numbers, the unit is "agents" and each header gets a tooltip "1 run, 3 agents". |
| F2 | **`r` means opposite things in two panes**: Resume in Mission control, Reject in the Inbox (and in P4.3 cards). Both are spec'd. | Keep both (different panes, never both focused). Every key-hint strip shows its own pane's labels, and the Inspector's actions never show `r`. A row at Needs you whose target is paused/suspended shows "Resume r"; a decision row never shows `r`. See §3. |
| F3 | **`o` on a run row.** Spec §3.4 gives `o` to outcome rows only. Inbox `o` = open, and a run row with an open PR has a PR link. | `o` on a run row with `pr` opens the PR; on a run row without one it does nothing (hint hidden). On a solo row: nothing. |
| F4 | **No keyboard way out of a taken-over session except leader `o`** (D5). A keyboard-only user who takes over to answer a prompt has `⌘+; o`. That works, but nothing on screen says so once the watch strip is gone. | Take-over keeps a thin strip (§5.2): "Typing in back-1 · ⌘+; o Mission control". Costs one 24 px line, removes the dead end. `ui` draws it; the brief's "hint in the header" becomes this strip. |
| F5 | **Queued members are counted twice on purpose** (chip in the run row and a row in Queued, spec §3.2). A run with 1 queued member and nothing else shows "Queued · 1 agent" and a chip with the same wait. | Fine, but the chip of a queued member shows only "queued ≈14:20" (no reason), and its Queued row carries the reason. The Queued row names its run: "ui-1 · shop-api › checkout". Keeps the lane the whole queue without repeating the sentence. |
| F6 | **The fixture has an empty Waiting on lane**: front-1 → back-1 is a member's wait, so it lives in the run row (D4). The Waiting on lane would show nothing in the demo. | Lanes with 0 rows are **hidden**, except Needs you ("Nothing needs you", so the absence is visible). The demo then shows Needs you, Running, Queued, Done today. Acceptance §6.1 already lists no Waiting on row. |
| F7 | **Shell at 1512 when the chat column exists** (ux-brief §2.1): the proposal in the brief holds, with the widths computed in §4. | Inspector overlays the chat column's slot below 1728, ⌘I toggles; docked ≥ 1728. In P4.2 code (no chat) it stays docked at ≥ 1512 as today. Needs a SPEC §9.6 edit when P4.3 lands: "Inspector (⌘I): docked at ≥ 1728 px, overlay below" (today: 1512). |

---

## 1. Copy

### 1.1 Lanes and counts (F1)

| Lane | Header | Empty / other forms |
|---|---|---|
| Needs you | "Needs you · 1 run" · "Needs you · 2 sessions" · "Needs you · 1 run · 1 session" | 0 rows: "Nothing needs you" (muted, lane kept) |
| Running | "Running · 1 session" · "Running · 2 runs · 3 sessions" | hidden at 0 |
| Waiting on | "Waiting on · 1 session" | hidden at 0 |
| Queued | "Queued · 2 agents" | hidden at 0 |
| Done today | "Done today · 4 outcomes" ("1 outcome") | hidden at 0 |
| Idle | "Idle · 3 sessions" ("1 session"), collapsed by default | hidden at 0 |

- **Overflow**: Done today shows 10 rows then "Show all 14" (button, expands in place; "Show fewer" when open).
- **Needs you, last line** (decisions with no session, D3): "2 decisions not tied to a session · Open in Inbox d". Singular: "1 decision not tied to a session". Once P4.3 lands the target is the chat: "… · Open in chat d". The line is a row (`j/k` reach it, `⏎` and `d` open it).
- **Collapsed lane**: header only, "Running · 2 sessions ▸", counts stay.

### 1.2 Phase stepper

Six steps, in this order and these labels: **Draft · Spec · Planning · Executing · Integrating · PR**. Text equivalent (aria, tooltip): "phase 4 of 6: executing". Step state and mark:

| Record `phase` | Current step | Mark on the current step | Text next to the stepper |
|---|---|---|---|
| draft · spec · planning · executing · integrating · pr | that step | none | "executing · 2 h" (phase label lower case + since) |
| spec_gate | Spec | "at gate" | "spec gate · 12 min" |
| pr_gate | PR | "at gate" | "PR gate · 3 min" |
| paused | the step the run paused in | "paused" | "paused · executing · 40 min" |
| deferred | the step it was deferred in | "deferred" | "deferred · planning · until Mon 09:00" if the core gives a time, else "deferred · planning" |
| failed | the step it failed in | "failed" (error color, ✕) | "failed · integrating · 5 min" |
| done | — (a done run is not a row; its outcome is in Done today) | — | — |

Steps before the current one are filled, after it hollow. Marks are text + icon (principle 8). "Since" is `phase_since_ms` in the relative format (P4.1 §1.1): "just now", "3 min", "1 h 12 min", "2 d".

### 1.3 Run row

```
shop-api › checkout   [Draft ✓][Spec ✓][Planning ✓][Executing ●][Integrating ○][PR ○]   executing · 2 h
3/7 tasks done · PR #12 ↗ ●(CI)                      blocker: back-1 · permission: Bash pnpm add zod · 2 min
[lead-1 ▶ Working] [back-1 ⚠ permission · 2 min] [front-1 ⧗ → back-1 · 12 min] [ui-1 ○ queued ≈14:20]
```

| Part | Copy |
|---|---|
| Title | `<project> › <feature>` |
| Tasks | "3/7 tasks done" (0 total: "no tasks yet"). Extras in the Inspector, not the row. |
| PR | "PR #12 ↗" (no number parsed: "PR ↗"), with the CI dot from `PrLine`; absent until `pr` |
| Blocker line | `blocker: <agent> · <reason> · <since>` using the core's composed reason (P4.1 §2). Absent when the run has no blocker. Run at `error` level: `blocker:` becomes `error:`. |
| Member chip | status icon + name + short wait. Short wait = the reason's first clause + since, cut to 28 chars with "…": running "lead-1 · Edit src/feed.ts", permission "back-1 · permission · 2 min", agent wait "front-1 → back-1 · 12 min", task wait "ux-1 · blocked by T-2 · 25 min", limit "back-1 · 5h full · resets 16:00", queued "ui-1 · queued ≈14:20" (F5), not started "ui-1 · not started", paused "back-1 · paused", error "back-1 · error · 2 min", idle "ux-1 · idle", suspended "ux-1 · suspended", done "back-1 · done". |
| Chip order | lead first, then the core's order. Selected chip (`h/l`): filled outline + the key hint "⏎ open back-1" in the toolbar hints. Default target is the blocker's agent (D6); the row shows it selected. |
| Overflow | past 6 chips: "+3 more" chip, `l` steps through the hidden ones and swaps them in. |

### 1.4 Solo row

`<project> › <session>` + status badge (P4.1 §6, "Working", "Waiting for you", "Blocked by limit", "Idle"…) + the P4.1 `WaitingChip` line + since. Running: "blog › fix-feed · Working · Edit src/feed.ts" (no since). Waiting for you: "blog › fix-feed · Waiting for you · question: which auth provider? · 4 min". Same strings as P4.1 §6 "Overview row".

### 1.5 Queue row

```
infra-1 · infra › deps     Queued · live cap 2/2 · 1st in line · ≈14:20 · 6 min     [Run now ≈ +2.1 pp week]  [Cancel]
```
- **Reason** is the record's `reason` (P4.1 §2.2); the badge label comes first (P4.1 F1). Position is in the reason ("2nd in line"); `queue.position` is not repeated. Paused agents: "Paused · by you · 1 h 5 min" with **Resume r** in place of Run now. Not started: "Not started yet · live cap 3/3 · ≈14:20 · 2 min" with **Start now**.
- **Tier**: shown only as a muted tag when not `normal` ("low", "high", "critical"), tooltip "Allocation tier: low". It is not in the reason (P4.1 §2.2).
- **Run now button**: "Run now ≈ +2.1 pp week" in the window `binds` names: week → "≈ +2.1 pp week"; five_hour → "≈ +6 pp 5h". Missing cost (`run_now` absent): "Run now" only. Start now: same with "Start now".
- **Cancel** removes the queue entry: "Cancel" with a tooltip "Removes it from the queue; the agent stays as it is". Confirms only when the entry is a run member ("Cancel ui-1? The checkout run keeps going without it.").
- **Member rows** carry the run's name after the agent: "ui-1 · shop-api › checkout" (F5).

### 1.6 Needs-you variants (kind → badge + reason)

Same strings as P4.1 §2.1/§6; this lists the badge, the extra action shown on the row and the mono key hint.

| Variant | Badge · reason (composed) | Row action |
|---|---|---|
| permission | Waiting for you · permission: Bash pnpm add zod · 2 min | "Open in Inbox d" (→ card after P4.3) |
| question | Waiting for you · question: which auth provider? · 4 min / "3 questions: Auth" | same |
| spec gate | At gate · spec gate · checkout · 12 min | same |
| PR gate | At gate · PR gate · checkout · 3 min | same |
| restart gate | At gate · restart gate · git status not clean · 1 min | same |
| Governor ask | Waiting for you · run now? ≈ +2.1 pp of the week · 1 min | same |
| trust / pane prompt | Waiting for you · trust dialog in its pane · 30 s / a prompt in its pane · 6 min | "Take over ⏎" (the answer is in the pane) |
| parked | Waiting for you · permission (parked): … · 22 min | same as permission |
| error | Error · API error 529 (overloaded) · 2 min | "Resume r" · "Open in Inbox d" |
| startup error | Error · no session after 30 s · 1 min | "Open in Inbox d" |

Errors sort after the blocking ones (D3) and the badge is the error one (never confusable with Waiting for you).

### 1.7 Outcome lines (Done today)

The P4.1 §5 `text` plus the time on the right; **seen and unseen look the same**. Two additions for Mission control: the PR link is part of the row ("PR opened · blog #12 ↗ · 15:30", `o` opens the PR), and a run's outcomes are not grouped. Examples from the fixture:

```
T-3 done · checkout API (back-1) · 14:02
Merged back-1 into integration · 14:10
PR opened · blog #12 ↗ · 15:30
checkout: planning → executing · 11:40
```
`o` on an outcome: PR → opens the link; task_done / merge_done → focuses the run's row; run_phase / run_done → the run; turn_done → the agent; report_ready → the report; sync_conflict → the task. Empty: "Nothing done yet today."

### 1.8 Row details (`space`)

One line per agent of the row, columns in this order (FR-901 kept): **agent · activity · elapsed · ctx % · usage · Governor · memory**. Header cells: "Agent", "Doing", "Running for", "Context", "Usage this session", "Governor", "Memory". Values: "Edit src/feed.ts" or the wait; "2 h 10 min"; "62 % ctx"; "+3.4 pp"; "Green · live" / "Queued · 2nd in line" / "—"; "210 MB" ("—" for suspended). A queued agent has no memory, ctx or usage: "—". Footer (Overview's, with the units): "2 need you · 3 running · 1 waiting · 2 queued · 1 error · 4 live / cap 3 · RAM 1.4 GB" (P4.1 §1.2). `space` again closes.

### 1.9 Toolbar, filter, footer

- **Toolbar left**: lane-independent title "Mission control". **Filter button**: "All projects" (no filter) · "2 projects of 4" · "shop-api" (one project: its name) · "No projects selected" never exists (the last project cannot be unchecked; Esc closes). **Right**: key hints "j k move · ⏎ open · space details · f filter" (what the row at hand allows; hidden ones are dropped, never greyed).
- **Filter popover** (`f`): a list with checkboxes, "All projects" first; each project with its counts ("shop-api · 1 needs you · 2 queued · 3 running", zero parts left out, P4.1 §1.2); keys `j/k` move, `space` toggles, `a` selects all, `⏎` or Esc applies and closes. Below the list: "Remembered for next time".
- **Hidden-rows notice** when a filter hides rows: the filter button turns to the accent style and reads "2 projects of 4"; tooltip "Hiding blog, infra · 3 running, 1 queued" (counts from the core's per-project counts). Nothing else changes: a hidden lane reads as empty, never as an error.
- **Lane vs filter when it hides all**: "Nothing in the 2 projects selected · Show all projects f".

### 1.10 Watch-mode strip and take-over strip

| State | Strip copy | Keys |
|---|---|---|
| Watching | "Watching back-1 · ⏎ take over · Esc back" (agent name in mono; the terminal is live, dimmed ring) | ⏎ / click in the terminal / Esc |
| Watching, agent gone (killed, done) | "back-1 has stopped · Esc back" (terminal frozen, last output stays) | Esc |
| Watching, agent paused | "Watching back-1 (paused) · ⏎ take over · Esc back" | same |
| Taken over (F4) | "Typing in back-1 · ⌘+; o Mission control" (muted, 24 px) | all keys to the PTY, leader works |
| Taken over, agent gone | strip turns to "back-1 has stopped · ⌘+; o Mission control" | — |

Tab title is the agent name; Mission control stays first in the tab bar. Re-opening a session already open as a tab focuses that tab and is again in watch mode.

### 1.11 Empty states

| State | Copy | Action |
|---|---|---|
| No projects | "No projects yet" · "Add a project to see what runs, waits and queues across them." | **Add project ⌘N** (J0) |
| Nothing running (projects exist, no rows at all, Done today may have rows) | "Nothing is running" · "Start a session or a feature and it shows up here." | **New session ⌘T** |
| Everything waits on you (only Needs you has rows) | the Needs you lane, then a muted line "Nothing else is moving" | — |
| Nothing at all today | "Nothing is running" and Done today "Nothing done yet today." | New session |
| Filter hides everything | §1.9 | Show all projects f |

### 1.12 Run now / Start now confirmation (FR-809)

Opened by `n` or the button. Dialog (⌘⏎ submits, Esc cancels), title **"Run infra-1 now?"** (Start: "Start infra-1 now?").

```
Run infra-1 now?
It is queued: live cap 2/2 · 1st in line · ≈14:20.
Running it now costs ≈ +2.1 pp of the week and ≈ +6 pp of the 5h window.
The week holds it: that is the window this is counted against.
                                         [Keep queued]   [Run now ≈ +2.1 pp week]
```
- Both windows are always shown; the binding window is named on the next line ("The 5h window holds it: …" when `binds` is five_hour) and its figure is the one on the button. Missing figure: that sentence is left out; no `run_now` at all: "Running it now uses a slot above the cap." only if the cap is the reason.
- Over the cap (interactive above it): add "It runs above the live cap."
- Cancel on a run member: see §1.5.

### 1.13 Toasts (only after the core confirms; P4.1 §4 rule)

A row action toasts when `agent_action` / `governor_act` returns ok. A pending or failed call shows the row's button busy, never a toast of success. Format: past tense + object.

| Action | Toast | On failure |
|---|---|---|
| Interrupt `i` | "Interrupted · back-1" | "Failed: <error, ≤ 80>" |
| Pause `p` | "Paused · back-1" | same |
| Resume `p`/`r` | "Resumed · back-1" | same |
| Suspend `s` | "Suspended · front-1 · resumes on demand" | same |
| Kill `⇧K` (after "Kill back-1? It loses its current turn. ⏎ kill · Esc cancel") | "Killed · back-1" | same |
| Run now / Start now | "Running now · infra-1 · ≈ +2.1 pp of the week" (P4.1 §4 ask) | same |
| Cancel queue entry | "Removed from the queue · infra-1" | same |
| Filter change | no toast | — |

Toasts name the **target agent**, so a run row's action is never ambiguous. A key on a row where the action is not valid (resume on a running agent) does nothing and shows no toast; the hint for it is not shown.

---

## 2. States matrix: Mission control

Fixture: ux-brief §4, Thu 29 Oct 14:00. "Hint" = what the toolbar's key hints show for that row.

| # | Element | State | What it shows | Hint / actions |
|---|---|---|---|---|
| 1 | Shell | normal | lanes Needs you · 1 run, Running · 1 session, Queued · 2 agents, Done today · 4 outcomes; Idle line hidden (0) | — |
| 2 | Needs you lane | run row with a blocker | checkout row (stepper at Executing, "3/7 tasks done", blocker back-1 permission, chips lead-1 / **back-1 selected** / front-1 / ui-1) | ⏎ open back-1 · i p s ⇧K act on back-1 · d open decision · h l chips · space details |
| 3 | Needs you lane | no decision session line | "2 decisions not tied to a session · Open in Inbox d" | ⏎ / d |
| 4 | Run row | no blocker (running) | stepper, tasks, chips; no blocker line | ⏎ open lead-1 (default target lead) |
| 5 | Run row | gate | stepper with "at gate" on Spec or PR; blocker line is the gate; the lead is the target | d · ⏎ open lead-1 |
| 6 | Run row | paused / deferred / failed | mark on the current step (§1.2); failed in error color; level error → lane Needs you after blocking rows | p resumes the target (paused only) |
| 7 | Run row | no PR / no tasks | PR part and task part left out ("no tasks yet" at 0 total) | — |
| 8 | Run row | selected chip | outline on the chip; row selection bar; Inspector follows (agent) | h back to the run: Inspector → run |
| 9 | Run row | details open (`space`) | one line per member below the chips (§1.8) | space closes |
| 10 | Solo row | running | "blog › fix-feed · Working · Edit src/feed.ts" | ⏎ open · i p s ⇧K |
| 11 | Solo row | waits (user / agent / limit / task / external) | badge + reason + since in the lane the level says | ⏎ open · d for decisions |
| 12 | Queue row | queued (`queue.position`) | §1.5 with "Run now ≈ +2.1 pp week" | n · Cancel (click, or ⌫ asks) |
| 13 | Queue row | not started | "Not started yet…" + "Start now"; no position | n |
| 14 | Queue row | paused | "Paused · by you" + "Resume" (no cost, no ETA) | r |
| 15 | Queue row | `run_now` missing | button "Run now" without cost | n confirms without the cost lines |
| 16 | Queue row | run member | adds "· shop-api › checkout" | n |
| 17 | Queue row | 5h-guard hold | reason "5h guard · 5h window at 86 % · ≈13:00"; button costs in 5h ("≈ +6 pp 5h") | n |
| 18 | Waiting on lane | solo/agent waits | as 11 | ⏎ |
| 19 | Idle line | collapsed | "Idle · 3 sessions ▸" | ⏎ / space expands |
| 20 | Idle line | expanded | solo rows with badge Idle / Suspended and no reason | ⏎ open · r resumes a suspended one |
| 21 | Done today | 4 outcomes | outcome rows §1.7 | o open |
| 22 | Done today | more than 10 | 10 rows + "Show all 14" | ⏎ on the button expands |
| 23 | Done today | empty | hidden; if everything is empty "Nothing done yet today." | — |
| 24 | Toolbar | no filter | "All projects" | f |
| 25 | Toolbar | filter active | "2 projects of 4" (accent) | f |
| 26 | Filter popover | open | §1.9 | j k space a ⏎ Esc |
| 27 | Footer | normal | §1.8 footer | — |
| 28 | Whole pane | no projects | §1.11 | ⌘N |
| 29 | Whole pane | nothing running | §1.11 | ⌘T |
| 30 | Whole pane | everything waits on you | Needs you + "Nothing else is moving" | — |
| 31 | Whole pane | filter hides all | §1.9 | f |
| 32 | Whole pane | 30 agents at 1280 | density check: rows 40 px; chips wrap to a second line after 4; details closed; lane scrolls inside the pane (no virtualization < 200 rows); footer pinned | j/k scroll to keep the row visible |
| 33 | Session tab | watch / take-over / gone | §1.10 | ⏎ Esc ⌘+; o |
| 34 | Row action | pending | the action's chip shows a busy state; the other keys stay live | — |
| 35 | Row action | failed | toast "Failed: …", the row unchanged | — |
| 36 | Core / app | no `now` yet (first 300 ms) | lane skeletons, no counts; never "Nothing is running" (that would be false) | — |

Selection rule (also for the Inspector): selection moves with `j/k` across rows; if the selected row leaves its lane (state changed), the selection follows the same run/agent to its new lane; if it disappears, the next row takes it.

---

## 3. Keyboard map

### 3.1 Mission control (pane-local, pane focused, not a terminal)

| Key | Action | On |
|---|---|---|
| `j` `k` / ↓ ↑ | Move row (across lanes, skipping headers and collapsed lanes) | any |
| `h` `l` / ← → | Move between member chips (the target agent) | run row |
| `⏎` | Open the target's session (watch mode); on the "Show all" button expands; on the no-session line opens it | row |
| `space` | Row details | row |
| `i` | Interrupt the target | agent with a live turn |
| `p` | Pause / resume the target | any agent |
| `s` | Suspend | idle or waiting agent |
| `r` | Resume | paused / suspended |
| `⇧K` | Kill (asks first) | agent |
| `d` | Open the row's decision (Inbox; card in P4.3) | row with a decision |
| `n` | Run now / Start now (confirms with the cost, §1.12) | queue row |
| `f` | Project filter | pane |
| `o` | Open: PR, run, agent, report (§1.7); PR of a run row (F3) | outcome row · run row with PR |
| ⌫ | Cancel the queue entry (confirms for run members) | queue row |
| `Esc` | Closes a popover or dialog; in a session tab in watch mode returns to Mission control; elsewhere nothing | — |

Other keys do nothing (no beep). Single keys fire only with no ⌘/⌥/⌃; `⇧K` is `shift+k`.

### 3.2 Conflict check (result)

| Against | Result |
|---|---|
| **Global ⌘ chords** (⌘K P B I N T W J , ⇧T ⇧I) | No conflict: Mission control keys are unmodified; `i`/`n`/`p`/`f` differ from ⌘I/⌘N/⌘P by the modifier. |
| **Leader keys** (`⌘+;` then s v h j k l z t ] [ e w p o g) | No conflict: the leader consumes the next key before the pane sees it. `⌘+; p` pauses the *focused* agent; pane-local `p` pauses the *selected target*: same effect on a different object, both documented in the key map. `⌘+; o` opens Mission control, the pane's own `o` opens an outcome. |
| **Inbox map** (`j k a r ⏎ o e x u`, `c` for the digest, P4.1 F7) | No technical conflict (panes differ). **`r` has opposite meanings** (Resume here, Reject there): F2. `⏎` replies there, opens a session here. `o` open in both. `d`, `n`, `f`, `space`, `i`, `s`, `h`, `l`, `p` are unused in the Inbox. `e` (open in nvim) is not bound in Mission control. `x`/`u` are not bound here. |
| **`k` vs `⇧K`** | `k` (no shift) moves up, `⇧K` kills. The handler tests `shift+k` before `k`; a caps-lock `K` is not `shift+k` and moves up. |
| **P4.3 cards** (FR-707 keys `a r ⏎ o x u` act on cards when the chat has focus) | No conflict: the chat is a separate focus target; ⌘⇧I focuses it, ⌘L the input. Mission control `d` is the bridge: it focuses that card in the chat. |
| **Terminal panes** (every key to the PTY) | Mission control keys never fire there. Watch mode: the terminal is dimmed and does not take keys; only `⏎`, `Esc` and a click are handled, plus ⌘ chords and the leader. After take-over `Esc` goes to Claude Code. |
| **Palette / dialogs** | The confirm dialog (§1.12, kill) takes ⌘⏎ / ⏎ to confirm and Esc to cancel before the pane sees a key. A focused text field (filter search is not in v1) swallows single keys. |
| **IME / dead keys** | Single keys are matched on `e.key` after composition; not tested with physical keys yet (R9). |

Result: **one semantic collision (F2) and no binding collision.** Penpot page 05 gets the table above plus the note "`r` = Resume in Mission control, Reject in the Inbox".

Palette entries (⌘K) for each row action, with the target in the label: "Pause back-1", "Open Mission control", "Filter Mission control by project".

---

## 4. Shell at 1512 with the chat column (F7)

Widths: sidebar 240, chat column 360, Inspector 320, centre minimum 640 (a run row with six chips needs ~600 plus the lane gutter).

| Viewport | Sidebar | Centre | Chat | Inspector |
|---|---|---|---|---|
| 1280 | rail 56 | 884 | 340 | overlay in the chat's slot |
| 1512 | 240 | 912 (Inspector closed) / 592 if docked | 360 | **overlay in the chat's slot** (not docked) |
| 1728 | 240 | 808 | 360 | **docked 320** |
| P4.2 code (no chat) | 240 / rail <1512 | the rest | — | docked ≥ 1512, overlay below (as today) |

At 1512 docking the Inspector would take the centre to 592, below the 640 a run row needs, and cut the chips to two per line. Decision: **below 1728 the Inspector is an overlay over the chat column's slot**, same width as the chat, opened with ⌘I (toggle), closed with ⌘I or Esc.

Rules annotated on the frame:
1. **Never auto-opens.** The Inspector follows the selection, but only when it is open. Opening it is a choice.
2. **Open cards stay visible.** The overlay replaces the chat only visually; the title badge ("2") and a one-line banner at the top of the overlay ("1 card needs you · ⌘⇧I") say what is behind it. A level 1–2 card posted while it is open also shows the banner; it does not close the overlay.
3. **⌘⇧I and ⌘L close the overlay** and focus the chat (the user is asking for the cards).
4. **Resize across 1728** keeps the user's open/closed state; docked and overlay open are the same "open".
5. In P4.2 nothing changes at 1512: the right column is the Inspector, docked. The overlay-over-chat variant is drawn now and built in P4.3.

---

## 5. Inspector variants

All three share the P1 header (kind icon, title, close ⌘I). Fields in order:

### 5.1 Agent
As P4.1 §6 "Inspector" column, in the P1 order: **Status** (badge) · **Waiting on** (kind link, reason, since; hidden when none) · **Doing** (activity) · **Task** · **Model** · **Context** (ctx %) · **Usage this session** (pp) · **Governor** · **Memory** · **Worktree**. Actions: Open session ⏎, Interrupt, Pause/Resume, Suspend, Kill. Nothing new.

### 5.2 Run
1. **Title**: `<feature>` and `<project>` ("checkout · shop-api").
2. **Phase**: the stepper (same marks as §1.2), then "executing · since 12:00 (2 h)".
3. **Blocker** (only if any): `<agent> · <reason> · <since>`, agent a link (selects the agent).
4. **Members**: one line each, lead first: status icon, name, role, short reason + since. A click selects the agent (Inspector switches to Agent, `h` in Mission control returns to the run).
5. **Tasks**: "7 tasks · 3 done · 2 in progress · 1 in review · 1 blocked" (zero parts left out; "blocked" = unmet depends_on). Each count opens the board filtered (`Open board`).
6. **PR** (when `pr`): `PrLine`: "PR #12 ↗ · CI passing".
7. **Started**: "Started 10:42 · 3 h 18 min ago".
Actions: **Open feature** (the feature pane), **Open board**, **Open PR ↗** (if any). No team-level pause (out of scope).

### 5.3 Decision
1. **Kind label + title** (P4.1 §3.2), 2. **Subject line** (P4.1 §3.1), 3. **Lifecycle marker** (P4.1 §3.4), 4. **Context blocks** (P4.1 §3.3, read-only here: they collapse past 6 lines like in the card), 5. **Actions**: "Open in Inbox d" (until P4.3, then "Open card d"). The Inspector never decides: no Allow/Deny buttons (the Inbox/card owns the undo window).

Shown when the selection is the no-session Needs-you line or a decision opened with `d`.

---

## 6. Session over the board: tab behaviour (annotations for page 04)
- ⏎ on a row creates (or focuses) the tab **after Mission control's** in its tab group; Mission control stays first and cannot be closed by Esc. Closing the tab (⌘W) returns to Mission control.
- Esc in watch mode: focus returns to Mission control's pane; the session tab stays in the group (its terminal keeps rendering).
- Watching never counts as an intervention (FR-1311); only typed input does, and the take-over strip is where that starts.

---

## 7. J12 storyboard content (page 06)

Five frames, one per SPEC J12 step, 1512 × 982 each, with a caption under the frame (**Step N · title**) and a "keys" line. Same fixture in every frame, cut to what the step shows. Chat column on the right (Part B), Mission control centre.

| # | Frame | What is drawn | Annotations (callouts) | Key sequence |
|---|---|---|---|---|
| 1 | **Morning line** · 08:30 | The chat's first entry: morning line "Good morning. 2 runs in progress, 1 decision waiting, week 38 % against 35 % planned." Mission control: Needs you · 1 run, Running · 1 session, Queued · 1 agent. Overwatch status "awake". | ① "The core starts the day's session with its briefing" ② "One line: runs, decisions, week against plan" ③ "Mission control already shows the same numbers: one state" | none: the app opens on Mission control. `⌘⇧I` if the user wants the waiting card. |
| 2 | **The day, settled** · 11:40 | Chat: two collapsed lines ("▸ allowed back-1: cargo add serde · policy installs", "▸ answered front-1 from api-contract.md §2 ↗ citation"), one restart line ("▸ restarted ui-1: first session never started"), then two open cards: **spec gate · checkout** with the Overwatch annotation "scope matches the brief" and **permission: WebFetch registry.internal** (out of policy) with "Overwatch recommends: deny, not in the allow list". Mission control: run row at "spec gate" (at gate mark). | ① "Routine prompts settle themselves: one line each, expand to see, undo while it still means something" ② "The Overwatch leaves the user only what policy can't settle: each card with its recommendation" ③ "Level 1 notifies; collapsed lines never do" ④ "Run row shows the same gate: the Needs you row and the card are one decision" | `⌘⇧I` focus the oldest open card · `a` approve spec · `j` next card · `r` reject the network call · `u` undo within 5 s (shown on the toast). |
| 3 | **Ask why** · 14:05 | User message "why is checkout slow today?" in the chat input, then the Overwatch answer: "checkout waited 52 min today: 38 min on the live cap (2/2) behind infra/deps, 14 min on back-1's permission." with a small list citing the attention records and the Governor log ("Governor log 11:02–11:40 ↗"). Status "working" then "awake". | ① "Ask in words: ⌘L focuses the input, ⏎ sends" ② "Answers are built from the attention records and the Governor's log, with links" ③ "Overwatch status: asleep → waking → working → awake" ④ "Cards still work if the Overwatch is down" | `⌘L` · type · `⏎` · `Esc` returns focus to the board. |
| 4 | **Third time, a rule** · 15:20 | Rule proposal card: "You allowed Bash `pnpm add` in shop-api three times. Add a rule: allow `pnpm add *` in shop-api?" with the three earlier decisions as links, the rule text in mono, buttons **Add rule a** · **Not now r** · **Edit ⏎**. After adding: marker "Added to shop-api policy · 15:21" and the next matching prompt as a collapsed line "▸ allowed back-1: pnpm add zod · rule from 15:21". | ① "Proposed after the third identical answer, never before" ② "One key adds it to the project's policy" ③ "The next matching call is a collapsed line: no card, no notification" ④ "Undo removes the rule" | `⌘⇧I` · `a` add rule (or click) · `u` undo within 5 s. |
| 5 | **Watch, don't open** · 16:00 | Mission control with back-1's row selected; then the session tab in watch mode with the strip "Watching back-1 · ⏎ take over · Esc back"; third mini-frame taken over with the strip "Typing in back-1 · ⌘+; o Mission control". Inspector (overlay at 1512) shows the agent. | ① "No pane needed: the row says what it waits on and since when" ② "A click on the row opens the session over the board" ③ "Watching does not take the keyboard; ⏎ does" ④ "Esc returns; the tab stays open" ⑤ "Leader o comes back from a take-over" | `j` to the row · `l` to a chip · `⏎` watch · `Esc` back · `⏎` `⏎` take over · `⌘+; o` Mission control. A click on the row replaces `j l ⏎`. |

Each frame also carries a one-line footer: "Interventions this day: 0" (FR-1311, G10) — the measure the journey exists to show. The frames are annotation-led: numbered callouts use the key-hint container for every key (`+` joins, a space separates steps, §9.7 notation).

---

## 8. Part B: decision cards, collapsed lines, rule proposal, morning line

Reuses P4.1 §3 whole: kind labels and titles (§3.2), context blocks (§3.3), lifecycle marker (§3.4), actions and keys (§7 matrix). A card is the Inbox detail laid in the 360 px column: header (kind label + age) · title · subject line · blocks · **Overwatch annotation** (if any) · actions · marker. Nothing in P4.1 is renamed.

### 8.1 Anatomy and the Overwatch annotation
- **Annotation**: one muted block above the actions, prefixed "Overwatch:" and at most two lines. Forms: "Overwatch: scope matches the brief", "Overwatch recommends: deny, not in the allow list", "Overwatch: no policy rule covers this". It may cite a doc: "per api-contract.md §2 ↗". Never an action: the user decides.
- **Marker** line at the foot (P4.1 §3.4), plus `Deciding… the Overwatch is looking at it` for *deciding by the Overwatch*.

### 8.2 Card per kind

| Kind | Blocks | Actions (key) |
|---|---|---|
| Permission | tool_call, annotation | Allow once a · Deny r · Deny with message ⏎ · Always allow · Open pane o |
| Question (1) | markdown, options, last_message | Answer ⏎ · Dismiss r · Open pane o |
| Question (n) | options ×n with "Question 2 of 3" | Answer ⏎ (enabled when each has a choice or text) |
| Gate · spec | markdown (rendered spec), docs, annotation | Approve spec a · Send back r · Send back with notes ⏎ · Open pane o |
| Gate · PR | markdown, diff_summary, annotation | Approve: push + open PR a · Reject r · Reply ⏎ |
| Gate · restart | markdown | Approve a · Reject r · Open o |
| Startup | error, pty_tail | Restart fresh · Dismiss · Open pane o (+ Delete after a failed fresh restart) |
| Ask (Governor) | markdown, cost, countdown | Run now a · Keep queued r · Open o |
| Budget | markdown, cost, countdown? | Apply a · Skip r |
| Trust | pty_tail | Open pane o |
| Wait | last_message, pty_tail | Open pane o |
| Error / alert | error, pty_tail or markdown | Open o · Dismiss · Restart fresh (spawn, startup) · Resume r (error) · Reconnect a (MCP) |
| Notion | markdown | Reply · Dismiss r · Open in Notion ↗ |

### 8.3 Lifecycle states of a card

| State | The card shows | Actions |
|---|---|---|
| Open | marker "Waiting for you · 4 min" | the kind's actions |
| Deciding by the Overwatch | marker "Overwatch is deciding…"; actions stay live (the user can decide first) | the kind's actions |
| Parked | "Parked · back-1 stopped; resumes on your answer" | permission actions minus "Answer in the pane" |
| Sent (undo window) | "Allowing once · Undo u" → "Sent to back-1 · 14:02"; blocks unchanged | none (Undo u while pending) |
| Effect | "Allowed · back-1 resumed · 14:02" (P4.1 §4 text); the card collapses to a line after 10 s | Open PR o (PR gate) |
| Failed | "Failed · push rejected: non-fast-forward" (error color) | the kind's actions again · Dismiss |
| Stale | "Stale · the agent moved on" (muted, blocks muted) | Dismiss |
| Dismissed | "Dismissed by you · 14:05", collapses | none |

**Threading under a run**: a follow-up card for the same run sits indented under the earlier answer ("↳ after: spec approved 14:02"); **many open cards**: newest at the bottom, the oldest open card focused first by ⌘⇧I; past 5 open cards the column shows "3 more open · j k" collapsed headers that expand on focus.

### 8.4 Collapsed decision line
One line, 28 px, no card chrome:
```
▸ allowed back-1: cargo add serde_json · policy: installs                     14:02
```
Pattern: `▸ <verb> <agent>: <short input or title ≤ 48> · <by>` with verb from `decided_by`/action (allowed, denied, answered, restarted, approved, applied, kept queued, dismissed) and `<by>` "policy: <rule>", "rule: <id>" or "Overwatch".
- **Collapsed**: the line. Never notifies, never counts in the badge.
- **Expanded** (`⏎` or click, `▾`): the original card, read-only, with its marker "Allowed by rule installs · 14:02" and the annotation.
- **Undo** (while it still means something: the 5 s window, longer for rule decisions where the effect is reversible): the line shows "Undo u" at the right and, expanded, a button. After it is used: "▸ allowed back-1: … · undone 14:03" (muted).
- **Undo expired**: "Undo" is gone and the expanded marker reads "Can't be undone: back-1 already ran it" (muted).
- `j/k` skips collapsed lines (SPEC §9.7); `⏎` on a focused one expands it.

### 8.5 Rule proposal card
Header "Rule proposal"; text "You allowed `pnpm add *` in shop-api 3 times. Add a rule: allow it in shop-api?"; the three earlier decisions as links; the rule in mono (`Bash(pnpm add:*)`, scope: project shop-api); actions **Add rule a** · **Not now r** · **Edit rule ⏎** (edit opens the policy text). States: open; added ("Added to shop-api policy · 15:21", muted, Undo u 5 s); declined ("Not now. Asking again after 3 more.", muted). Never more than one proposal open per rule.

### 8.6 Morning line and Overwatch status
- **Morning line** (core): "Good morning. 2 runs in progress, 1 decision waiting, week 38 % against 35 % planned." Parts left out at zero ("No runs in progress."). One line, no card, first entry of the day; older days are searchable (FR-1305).
- **Outcome digest entry**: P4.1 §5 rows grouped as "Today · 5 outcomes · Clear c" at the end of the thread.
- **Overwatch status** (chat header, right of "Overwatch"): **Asleep** (muted: "asleep · wakes on the next event"), **Waking** ("waking…"), **Working** ("working · 2 events"), **Down** (warning: "Overwatch is down · cards still work", with "Restart" button). In every state the cards and their keys work: the status never disables an action.
- **Chat input**: placeholder "Ask the Overwatch…" (⌘L); sending while asleep shows "waking…"; down: the input stays enabled, the message is kept, "Sent when the Overwatch is back".
