# P4.1 design notes: copy and states for "One now"

UX pass for P4.1 (One now), 2026-10-29, by `ux` for `ui` and `front`. Sources: [spec](../../.orchestrator/features/p-4-1-one-now/spec.md) §3.1 and §3.5, [UX brief](../../.orchestrator/features/p-4-1-one-now/ux-brief.md), [API contract](../../.orchestrator/features/p-4-1-one-now/api-contract.md), SPEC §9.1, §9.5 and §9.6, `QueueReason` (crates/governor/src/queue.rs), and the copy that ships today (`ui/src/lib/status.js`, `ui/src/lib/inbox.js`, `crates/orch-app/src/inbox.rs`).

P4.1 adds no screen and changes no layout. These notes give the copy the core writes (`reason`, decision `title`, outcome `text`), the copy the UI writes (labels, markers, toasts), and the states each existing surface must show. `ui` draws the components of ux-brief §3 from them (§9 below lists what each component must cover).

The P4.2 part (Mission control) is in [p4-2-design-notes.md](p4-2-design-notes.md).

Copy rules carry over from P1/P3: sentence case, no exclamation marks, name the agent and the project, pp for usage, local 24 h times, and **every number says what it is** (principle 11).

---

## 0. Findings for lead

All findings are accepted: F1–F7 as D11/D12 (4eec296), F8–F12 as D13 (6c8f895).

| # | Finding | Proposal | Owner |
|---|---|---|---|
| F1 | The brief's examples put the state word in `reason` ("Queued · …", "Not started · …", "Paused by you", "Waiting · penpot …", "Starting · 12 s"). Every surface also shows the status badge next to the reason, so the agent header would read "Queued · Queued · live cap 3/3". | **The reason never repeats the status label.** The core writes only what the agent waits on; the surface puts the badge label before it. The composed line is exactly SPEC §9.5's "Shown as" column: "Queued · live cap 3/3 · 2nd in line · ≈14:20". Text without a badge (`orchd now`, macOS notifications, tooltips) prints `<status label> · <reason>`. §1.1 has the rule, §2 the strings. | lead → back |
| F2 | Some examples put a duration in `reason` ("→ back-1 · message · 12 min", "Waiting · penpot execute_code · 7 min", "Starting · 12 s"), and the UI also appends `since`. That gives two durations, and the one in `reason` goes stale between two 30 s ticks. | **No duration in `reason`.** The UI appends the duration from `waiting_on.since_ms` (else `since_ms`) with the 30 s relative clock. Clock times (≈14:20, resets 16:00) stay in `reason`. Fixed numbers are fine too ("after 30 s"). | lead → back |
| F3 | `run_phase`, `run_done` and `merge_done` outcomes have no `text`, and the contract has no `feature` on an outcome. The digest row "checkout: planning → executing" needs the feature name, and runs done earlier today may not be in `runs`. | Add an optional `text` to **every** outcome, written by the core with the patterns in §5, plus an optional `feature`. The row renders `text` and needs no lookup. This is additive, so the contract allows it. | lead → back |
| F4 | Two P3 level-4 items have no home once `info-<turn>` items and level 4 move to outcomes: the weekly report digest (`payload.digest = weekly_report`, P3 J8 · 4) and the Linear/GitHub sync conflict (`action = task_conflict`). Neither is a contract outcome kind, and `info` is not a decision kind. | Two outcome kinds: `report_ready {report}` (`report` = "2026-W44": "Weekly report for week 44 is ready", `o` opens it) and `sync_conflict {task, backend}` ("ENG-140 changed here and in Linear: kept Linear's", `o` opens the task). | lead → back |
| F5 | `status.js` has no `not_started` entry, and `tokens.json` has no `color.status.not-started`. The record's `status` can be `not_started`. | `ui` adds the token and the icon (label "Not started yet"). It must differ from Queued: no process yet, so a hollow glyph is better than the queued hourglass. | ui |
| F6 | The **Start now / Run now** cost ("≈ +1 pp") is not in the record. The current strip reads it from `queue_entry`. | No contract change: the cost is an action detail, not attention state. The strip keeps reading `app_state.agents[].queue_entry` for the button (raw data stays for details, contract §3.2). A queued or not-started record's `reason` carries no cost. A Governor **ask** is different: it is a decision, its payload has the cost, and its needs_you reason says it (§2.1). | front (note only) |
| F7 | The digest **Clear** action needs a key. | `c` in the Inbox when the digest is focused (free in the Inbox map: a r ⏎ o e x u j k). Palette: "Clear today's digest". front checks for conflicts. | front |
| F8 | Contract §2.1 `effect` has no value for a budget answer or a startup Delete, and §4 only toasts on `effect`/`failed`, so "Applied · …", "Skipped · …" and "Deleted · front-1" would never show. | Add `applied`, `skipped`, `deleted` to `effect` (additive). | lead → back |
| F9 | Contract §4 drops P4.0 actions the Inbox ships today (`inbox.js` `actionLabels`/`keyIntent`): error `Resume r`, MCP alert `Reconnect a` and `Open Settings o` (`reconnect`, `open`), team question `Dismiss r` (`reject`), Notion `Open in Notion ↗`. §6 row 17 and §7 rows 3, 14, 15 use them. The contract's example label "Allow" is also "Allow once" in P4.0. | Add `resume` (error), `reconnect` + `reject` (MCP alert), `reject` (question), `open` (notion) to §4, and use "Allow once". | lead → back |
| F10 | Spec §3.1 Counts says `queued` = Governor queue length + not-started + paused. A Governor ask's agent is in the queue but at needs_you (rule 2 wins), and paused and most not-started agents also hold a queue entry, so that sum double-counts and disagrees with "agents at level queued". | Define `queued` only as "agents at level queued" (as contract §1 does) and drop the sum. | lead |
| F11 | Spec §3.1 run record and §3.5 use `run_id`, `feature_id`, `project_id`; contract §1 (final names) uses `run`, `feature`, `project`. Spec §3.1 rules 4–6 and rule 7 still hold the superseded reason examples ("Paused by you", "Starting · 12 s", "· 12 min"). | Align the spec on the contract's names and on D11. | lead |
| F12 | Three `QueueReason` strings of §2.2 need data the reason does not carry: `HardCap` (no allocation name, used or cap pp), `FiveHourGuard` (no F reading), `DeferredUntilReset` (no reset time). | back adds the fields to the variants (or reads them from the Governor snapshot when it writes the reason); else the part is left out ("checkout cap reached", "5h guard", "deferred to the weekly reset"). | lead → back |

---

## 1. Composition rules

### 1.1 The waiting-on line
```
[status badge: icon + label]  <reason>  · <since>
```
- **Status label**: SPEC §9.5 from the record's `status`, plus "Not started yet" for `not_started`.
- **Reason**: the core's string (§2). It never starts with the status label (F1) and holds no duration (F2).
- **Since**: the duration from `waiting_on.since_ms` (fall back to `since_ms`) in the relative-time format: "just now" under 1 min, "3 min", "1 h 12 min", "2 d". It is shown for needs_you, waiting, queued and error. It is not shown for running, idle or done: running shows `activity` instead, and idle/done show nothing.
- **Running**: `[Working] <activity>`, e.g. "Working · Edit src/feed.ts". `activity` is "writing" between tool calls.
- **Compact** (sidebar tooltip, rail tooltip, Overview row): icon + reason + since on one line. When it overflows, the reason gets the ellipsis and since stays visible. The full text is on hover.
- **Full** (agent header, Inspector): label + reason + since. The `ref` is a link: a decision opens its Inbox detail, an agent focuses that agent, a task opens its Board card.

### 1.2 Counts
Each figure carries its label, never a bare number in parentheses:

| Where | Copy | Source |
|---|---|---|
| Title badge | "2" in the dock badge, tooltip "2 decisions need you" | `counts.decisions` |
| Sidebar feature badge | "2 need you" (hidden at 0) | run `counts.needs_you` |
| Icon rail project | dot + "2", tooltip "shop-api · 2 need you · 1 queued · 3 running" | project counts |
| Overview footer | "2 need you · 3 running · 1 waiting · 2 queued · 1 error · 4 live / cap 3" | global counts |
| Overview filter chips | "Needs you 2" · "Running 3" · "Waiting 1" · "Queued 2" · "Idle 0" · "Error 1" | global counts |
| Status bar | "▶ 4 live / cap 3" · "⧗ 2 queued" | `live`, `live_cap`, `queued` |
| macOS notification (level 1, batched) | "3 agents need you" | `counts.needs_you` |

"1 need you" is wrong: it is "1 needs you" (singular) and "2 need you" (plural). The same goes for "1 decision" and "2 decisions".

---

## 2. Reason strings (core-written)

### 2.1 By `waiting_on.kind`

| Level · kind | Reason pattern | Composed line (badge label + reason + since) |
|---|---|---|
| needs_you · user · permission | `permission: <tool> <short input>` | "Waiting for you · permission: Bash pnpm add zod · 2 min" |
| needs_you · user · question | `question: <first line, ≤ 60 chars>` | "Waiting for you · question: which auth provider? · 4 min" |
| needs_you · user · several questions | `<n> questions: <first header>` | "Waiting for you · 3 questions: Auth · 4 min" |
| needs_you · user · spec gate (Lead) | `spec gate · <feature>` | "At gate · spec gate · checkout · 12 min" |
| needs_you · user · PR gate (Lead) | `PR gate · <feature>` | "At gate · PR gate · checkout · 3 min" |
| needs_you · user · restart gate | `restart gate · <detail, ≤ 40>` | "At gate · restart gate · git status not clean · 1 min" |
| needs_you · user · Governor ask | `run now? ≈ +<x> pp of the week` | "Waiting for you · run now? ≈ +2.1 pp of the week · 1 min" |
| needs_you · user · trust | `trust dialog in its pane` | "Waiting for you · trust dialog in its pane · 30 s" → "just now" |
| needs_you · user · pane prompt | `a prompt in its pane` | "Waiting for you · a prompt in its pane · 6 min" |
| needs_you · user · parked permission | `permission (parked): <tool> <short input>` | "Waiting for you · permission (parked): Bash pnpm add zod · 22 min" |
| waiting · agent · review | `→ <reviewer> · review of T-<n>` | "Idle · → lead-1 · review of T-4 · 3 min" |
| waiting · agent · plan | `→ <lead> · plan for T-<n>` | "Idle · → lead-1 · plan for T-6 · 8 min" |
| waiting · agent · message / handoff | `→ <member> · message` / `→ <member> · handoff` | "Idle · → back-1 · message · 12 min" |
| waiting · agent · Lead on members | `→ <a>, <b> · T-<n>, T-<m>`, then `+<k> more` past 2 | "Idle · → back-1, front-1 · T-3, T-4 · 40 min" |
| waiting · task | `blocked by T-<n> (<owner>, <task status>)` | "Idle · blocked by T-2 (back-1, in progress) · 25 min" |
| waiting · limit | `<window> full · resets <time>` | "Blocked by limit · 5h window full · resets 16:00 · 18 min" |
| waiting · startup | `starting · times out after 30 s` | "Working · starting · times out after 30 s · just now" (the 30 s relative clock can't count seconds; the fixed limit says how long it may take) |
| waiting · external | `waiting on <server> <tool>` | "Working · waiting on penpot execute_code · 7 min" |
| queued · governor | §2.2 | "Queued · live cap 3/3 · 2nd in line · ≈14:20 · 6 min" |
| queued · not started | §2.2 with no position | "Not started yet · live cap 3/3 · ≈14:20 · 2 min" |
| queued · paused | `by you` / `Pause all` | "Paused · by you · 1 h 5 min" · "Paused · Pause all · 10 min" |
| error | `<the error's one line>` | "Error · API error 529 (overloaded) · 2 min" |
| error · startup | `no session after 30 s` · `<mode> start exited (status <n>)` · `restarted fresh, failed again` | "Error · resumed start exited (status 1) · 1 min" |
| idle | no reason (empty) | "Idle" |
| idle · suspended | no reason | "Suspended" |
| done | no reason | "Done" |
| running | `activity`, not reason | "Working · Bash cargo test" |

Notes:
- **Short input** for permissions and activity: Bash = the command's first line, Edit/Write/Read = the path relative to the worktree, WebFetch = the host, MCP = `<server> <tool>` (from `mcp__<server>__<tool>`). Cut at 60 characters with "…".
- **Times**: today "16:00". Another day within 6 days "Mon 09:00". Further away "2 Nov 09:00".
- **Task status words**: backlog, ready, in progress, review, done (task palette, not agent palette).
- **Windows**: "5h window" and "week". A model window is "opus week".

### 2.2 Governor `QueueReason`

Pattern: `<reason>[ · <position>][ · ≈<estimated start>]`
- **Position**: "next" when 1st, else "2nd in line", "3rd in line"… It is left out for not-started agents (they wait for a slot, not a turn), for paused agents, and for reasons that carry their own time.
- **≈ETA**: from `estimated_start`, left out when unknown and when the reason already has a time (`resets`, `starts`).
- The **tier** is not shown, except in RedTier, where it is the reason.

| `QueueReason` | Reason | Example (composed) |
|---|---|---|
| `LiveCap {state: green, cap}` | `live cap <live>/<cap>` | "Queued · live cap 3/3 · 2nd in line · ≈14:20" |
| `LiveCap {state: amber·red·catch_up, cap}` | `live cap <live>/<cap> (<State>)` | "Queued · live cap 1/1 (Red) · next · ≈14:05" |
| `RedTier` | `Red · <tier> priority waits` | "Queued · Red · low priority waits · ≈Mon 09:00" |
| `OffHours` | `outside working hours` | "Queued · outside working hours · ≈Fri 09:00" |
| `FiveHourGuard` | `5h guard · 5h window at <F> %` | "Queued · 5h guard · 5h window at 86 % · ≈13:00" |
| `LimitReached {window: five_hour}` | `5h window full · resets <time>` | "Queued · 5h window full · resets 16:00" |
| `LimitReached {window: weekly}` | `week full · resets <time>` | "Queued · week full · resets Mon 09:00" |
| `DeferredUntilReset` | `deferred to the weekly reset · <time>` | "Queued · deferred to the weekly reset · Mon 09:00" |
| `HardCap` | `<allocation> cap reached · <used> of <cap> pp` | "Queued · checkout cap reached · 12 of 12 pp" |
| `PausedAll` | `Pause all` (status `paused`) | "Paused · Pause all" |
| `PausedAgent` | `by you` (status `paused`) | "Paused · by you" |
| `MemoryPressure {cap}` | `memory pressure · live cap <live>/<cap>` | "Queued · memory pressure · live cap 2/2 · next" |
| `BehindQueue` | `behind higher-priority work` | "Queued · behind higher-priority work · 3rd in line · ≈14:40" |
| `ScheduledOffHours {starts_at}` | `off-hours · starts <time>` | "Queued · off-hours · starts 18:00" |
| `ModelWindow {exhausted: false}` | `<model> week in Red · starts after reset <time>` | "Queued · opus week in Red · starts after reset Mon 09:00" |
| `ModelWindow {exhausted: true}` | `<model> week full · starts <time>` | "Queued · opus week full · starts Mon 09:00" |

Changes from the shipped copy (`status.js` `queueReason`): "live-agent cap 3 (red)" becomes "live cap 1/1 (Red)", so the cap is shown against the live count. "low priority (Red)" becomes "Red · low priority waits". "paused (p resumes)" becomes "Paused · by you", and the `p` hint moves to the strip's action. P3's "opus week exhausted" becomes "opus week full", which matches "5h window full". `<live>` is the Governor's live autonomous count (snapshot `live_autonomous`, what the cap is compared to), not `counts.live`, which also counts interactive sessions. `<F>` is the 5h reading the guard used (F12).

A **not-started** agent uses the same reason with no position: "Not started yet · live cap 3/3 · ≈14:20". An agent created with no queue entry yet reads "Not started yet · waiting for a slot".

---

## 3. Decisions (Inbox detail)

### 3.1 Subject line (UI)
`<agent> · <role> · <project> › <feature> · T-<n>`, leaving out the parts that are absent.
- "back-1 · Backend · shop-api › checkout · T-4"
- Solo: "fix-feed · blog"
- No agent (budget, battery, MCP): the project only, or nothing.

### 3.2 Kind label and title pattern (core-written `title`; the label is UI)

| Kind | Kind label | Title pattern | Example title · summary |
|---|---|---|---|
| permission | Permission | `<tool> · <short input>`; summary = the call's description | "Bash · pnpm add zod" · "Add zod" |
| permission (parked) | Permission · parked | same | "Bash · pnpm add zod" · "back-1 stopped; it resumes on your answer" |
| question (ask_user, 1 question) | Question | `<the question, ≤ 80>` | "Which auth provider?" · "" |
| question (AskUserQuestion, n) | Question | `<first question, ≤ 60> · <n> questions` | "Which auth provider? · 3 questions" · "" |
| gate spec | Spec gate | `Spec gate · <feature>` | "Spec gate · checkout" · first line of the spec summary |
| gate pr | PR gate | `PR gate · <feature>` | "PR gate · checkout" · "orch/checkout/integration → main · 7 files changed · +142 −9 lines" |
| gate restart | Restart gate | `Still waiting after a restart · <agent>` | "Still waiting after a restart · back-1" · the detail |
| startup | Startup | `<agent> did not start` · `<agent> could not start fresh` | "front-1 did not start" · "No session after 30 s: a dialog may be waiting in its pane" |
| ask (Governor) | Run now? | `Run <agent> now? ≈ +<x> pp of the week` | "Run back-1 now? ≈ +2.1 pp of the week" · "Queued · live cap 3/3 · default: keep queued in 18 min" |
| budget | Budget | `<reason, one line>` | "Reserve for your own work used up" · "Agents are squeezed harder until Mon 09:00" |
| trust | Trust dialog | `Claude Code asks to trust this folder` | same · "Answer it in back-1's pane: the app never does" |
| wait | Prompt in pane | `A prompt in <agent>'s pane` | "A prompt in front-1's pane" · its last message's first line |
| error · API | Error | `<agent> · API error <status> (<short why>)` | "back-1 · API error 529 (overloaded)" |
| error · spawn | Error | `<agent> could not resume` | "back-1 could not resume" · "claude exited: session file missing" |
| error · watchdog | Error | `<agent> · no progress for <n> min` / `<agent> over its turn ceiling · <x> of <cap> pp` | "front-1 · no progress for 20 min" |
| error · agent stopped | Error | `<agent> stopped with an error` | as today |
| alert · battery | Alert | `Off-hours work needs AC power` | "3 off-hours tasks won't run while the Mac sleeps" |
| alert · MCP auth | Alert | `<Server> MCP needs authentication` | "Penpot MCP needs authentication" · "ux-1 and ui-1 will fail its tools" |
| alert · worktree missing | Alert | `<agent>'s worktree is missing` | "back-1's worktree is missing" · the path |
| alert · merge in progress | Alert | `<op> in progress in <agent>'s worktree · <n> unmerged files` | "Merge in progress in back-1's worktree · 3 unmerged files" |
| alert · work in progress at restart | Alert | `<agent> had work in progress at the restart` | the why |
| alert · repair failed | Alert | `<agent>'s worktree could not be repaired` | the failure |
| notion | Notion comment | `Comment on <feature> · <doc>` | "Comment on checkout · spec.md" · "{author}: {text}" |

Actions keep their P4.0 labels (`inbox.js` `actionLabels`). The ask's actions are "Run now `a`" and "Keep queued `r`".

### 3.3 Context blocks: labels (UI)

| Block | Header | Body copy |
|---|---|---|
| tool_call | `<tool> in <worktree name>` | input in mono, collapsed past 6 lines: "Show 14 more lines" |
| markdown | none, or `From <source>` | rendered |
| docs | "Docs · 3 files" | one row per file: path · "142 lines" |
| diff_summary | "7 files changed · +142 −9 lines" | per file: path · bar · "+40 −3"; past 12 files, "Show 9 more files"; "Show patch" (loading: "Loading patch…"; error: "Could not load the patch: <why>") |
| options | "Question 1 of 3 · Auth" (header when given) | the question; options as radio (single) or checkboxes (multi), each "label — description". Multi: "Pick any". |
| last_message | "Last message · 14:02" | the text, 6 lines then "Show more" |
| pty_tail | "In its pane · last 12 lines" | mono |
| cost | "Cost" | "≈ +2.1 pp of the week · +6 % of the 5h window · done ≈14:40". Missing parts are left out. |
| countdown | none | "Keeps queued in 18 min" / "Applies in 18 min" (the default's verb) · past deadline "Applying the default…" |
| error | "Error" | the text, then detail in mono, collapsed |

### 3.4 Lifecycle marker (UI)

One line under the title, never a spinner. The text uses `lifecycle` fields only.

| State | Marker | Example |
|---|---|---|
| open | `Waiting for you · <since>` | "Waiting for you · 4 min" |
| parked | `Parked · <agent> stopped; resumes on your answer` | "Parked · back-1 stopped; resumes on your answer" |
| decided (in the 5 s undo window, client) | `<pending verb> · Undo u` | "Allowing once · Undo u" |
| decided (by someone else, before sent) | `Decided <by> · sending` | "Decided by rule git-safe · sending" |
| sent | `Sent to <agent> · <time>` · PR: `Pushing <branch>…` | "Sent to back-1 · 14:02" |
| effect | the effect text of §4, then `· <time>` | "Allowed · back-1 resumed · 14:02" |
| failed | `Failed · <error>` | "Failed · push rejected: non-fast-forward" |
| stale | `Stale · <stale_why>` | "Stale · the agent moved on" |
| dismissed | `Dismissed <by> · <time>` | "Dismissed by you · 14:05" |
| interrupted | `Interrupted · <agent> stopped before it was answered` | as written |

`<by>` from `decided_by`: user "by you", pane "in the pane", `rule:<id>` "by rule <id>", expired "when the hold expired", core "by the app", policy "by policy", overwatch "by the Overwatch".

---

## 4. Toasts, by effect

A toast shows only on a lifecycle push whose state is `effect` or `failed` (or `parked`). The one exception is the undo toast, which says what *will* be sent ("Allowing once · back-1 · Undo u", P4.0 `pendingToast`, unchanged). A decision decided elsewhere toasts only if P4.0 did (`outcomeToast`). Format: past tense + object, then what happened to the agent.

| Effect | Kind · action | Toast |
|---|---|---|
| allowed | permission · approve | "Allowed · back-1 resumed" |
| allowed | permission · always | "Always allowed · Bash(pnpm add:*) · back-1 resumed" |
| denied | permission · reject | "Denied · back-1" |
| denied | permission · reply | "Denied · reason sent to back-1" |
| resumed | question · answer / reply | "Answer sent · front-1 resumed" |
| resumed | parked permission · approve | "Allowed · back-1 resumed" |
| resumed | gate spec · approve | "Spec approved · lead-1 resumed" |
| resumed | gate spec · reply / reject | "Spec sent back with your notes · lead-1 resumed" |
| resumed | gate pr · reject / reply | "PR gate sent back · lead-1 resumed" |
| resumed | gate restart · approve | "Approved · back-1 resumed" |
| resumed | ask · approve | "Running now · back-1 · ≈ +2.1 pp of the week" |
| delivered | question / wait · reply, agent busy | "Answer delivered · front-1 reads it next" |
| queued | question · answer, agent queued | "Answer queued · front-1 starts ≈14:20" |
| queued | ask · reject | "Kept queued · back-1 starts ≈14:20" |
| queued | gate spec · approve, Lead queued | "Spec approved · lead-1 starts ≈14:20" |
| pr_opened | gate pr · approve | "PR opened · checkout #12 ↗" (o opens it) |
| restarted | startup / error · restart_fresh | "Restarted fresh · front-1" |
| dismissed | any · dismiss | "Dismissed · <title, ≤ 40>" |
| dismissed | MCP alert · reject | "Dismissed until it fails again · Penpot" |
| (budget) | budget · approve / reject | "Applied · <title>" / "Skipped · <title>" |
| (delete) | startup · delete | "Deleted · front-1" |
| failed | any | "Failed: <error, ≤ 80>" (stays until dismissed; o opens the decision) |
| parked (by expiry) | permission | "Parked · back-1 stopped and resumes on your answer" |

"≈14:20" comes from `effect_detail`, and the PR URL from `effect_detail` for `pr_opened`. Where an ETA is missing: "Answer queued · front-1".

---

## 5. Outcome rows (digest, level 4)

The row is the kind icon, the core's `text` (F3), the agent for task outcomes, and the time on the right.

| Kind | `text` pattern | Example row |
|---|---|---|
| task_done | `T-<n> done · <title>` | "T-4 done · checkout API (back-1) · 14:02" |
| merge_done | `Merged <branch tail> into <into tail>` | "Merged back-1 into integration · 14:10" (`orch/checkout/back-1` → "back-1"; an integration branch merged into `main` names its feature: "Merged checkout into main") |
| pr_opened | `PR opened · <feature> #<n> ↗`; solo: `PR opened · <project> › <agent> #<n> ↗` | "PR opened · checkout #12 ↗ · 15:30" |
| run_phase | `<feature>: <from> → <to>` (phase labels, lower case) | "checkout: planning → executing · 11:40" |
| run_done | `<feature> done` | "checkout done · 16:05" |
| turn_done (solo) | `<project> › <agent>: <summary first line>` | "blog › fix-feed: Fixed the feed pagination and added a test · 13:20" |
| report_ready (F4) | `Weekly report for week <n> is ready` (`<n>` from `report`, "2026-W44" → 44) | "Weekly report for week 44 is ready · 09:00" |
| sync_conflict (F4) | `<task> changed here and in <backend>: kept <backend>'s` (backend label: Linear, GitHub) | "ENG-140 changed here and in Linear: kept Linear's · 10:12" |

A run moving to `failed` is a run_phase row: "checkout: integrating → failed".

**Digest header**: "Today · 5 outcomes · Clear c". **Empty** (no outcomes today): "Nothing done yet today." **Cleared** (all seen): "All caught up · 5 done today". The number is `done_today` and survives Clear.

**Run phase labels** (run phase palette): Draft, Spec, Spec gate, Planning, Executing, Integrating, PR gate, Done, Paused, Deferred, Failed.

---

## 6. States matrix: agent surfaces

The fixture is Thu 29 Oct 14:00, shop-api › checkout `executing`, live cap 3 (Green), and solo blog › fix-feed. Each row is one record. The cells show what each surface renders from it. "Compact" and "full" are the chip forms of §1.1. **Dot** is the level color (attention level palette), and the status icon comes from the agent status palette.

| # | Record (level · kind · status) | Sidebar (agent row; feature badge) | Icon rail (project) | Overview row | Agent header + strip | Inspector |
|---|---|---|---|---|---|---|
| 1 | needs_you · user · waiting_for_you (permission) | status icon + "back-1" + "2 min"; tooltip compact; feature badge counts it ("1 needs you") | counts in `needs_you`; dot needs_you | badge Waiting for you · "permission: Bash pnpm add zod" · "2 min" | full chip; strip: permission card (P4.0) with Allow / Deny | Status: Waiting for you · Waiting on: **decision** "Bash · pnpm add zod" (link) · since 13:58 (2 min) |
| 2 | needs_you · user · waiting_for_you (parked) | as 1, "22 min" | as 1 | "permission (parked): Bash pnpm add zod" · "22 min" | strip: "Parked · back-1 stopped; resumes on your answer" + Allow / Deny | as 1, marker "parked" |
| 3 | needs_you · user · waiting_gate (spec gate, Lead) | lead-1 row; feature badge "1 needs you"; run label "Spec gate" | as 1 | badge At gate · "spec gate · checkout" · "12 min" | full chip; strip: "Spec gate · checkout · Open in Inbox o" | Waiting on: decision "Spec gate · checkout" |
| 4 | needs_you · user · waiting_for_you (pane prompt) | as 1 | as 1 | "a prompt in its pane" · "6 min" | strip: "A prompt below · answer it in the pane" | Waiting on: its pane |
| 5 | running · — · working | status icon, no duration | counts in running; dot running only if nothing higher | badge Working · "Edit src/feed.ts" | badge + activity; no strip | Status: Working · Doing: Edit src/feed.ts |
| 6 | waiting · agent · idle | "front-1" + "3 min"; tooltip "→ lead-1 · review of T-4 · 3 min" | counts in waiting | badge Idle · "→ lead-1 · review of T-4" · "3 min" | full chip; ref "lead-1" focuses it; no strip | Waiting on: **agent** lead-1 · review of T-4 · since 13:57 |
| 7 | waiting · task · idle | "ux-1" + "25 min" | counts in waiting | "blocked by T-2 (back-1, in progress)" · "25 min" | full chip; ref "T-2" opens the card | Waiting on: **task** T-2 (back-1, in progress) |
| 8 | waiting · limit · blocked_by_limit | status icon + "18 min" | counts in waiting | badge Blocked by limit · "5h window full · resets 16:00" · "18 min" | strip: "5h window full · resets 16:00 · the CLI's auto-continue was cancelled" | Waiting on: **limit** 5h window · resets 16:00 |
| 9 | waiting · startup · working | status icon + "just now" | counts in waiting | badge Working · "starting · times out after 30 s" · "just now" | strip: none until 30 s; then the startup error (row 18) | Waiting on: **startup** · since 13:59:50 |
| 10 | waiting · external · working (≥ 5 min) | status icon + "7 min" | counts in waiting | badge Working · "waiting on penpot execute_code" · "7 min" | full chip; strip: "penpot execute_code has run 7 min · it may wait on a browser tab · Open pane o" | Waiting on: **external** penpot execute_code · since 13:53 |
| 11 | queued · governor · queued | "back-2" + "6 min" | counts in queued | badge Queued · "live cap 3/3 · 2nd in line · ≈14:20" · "6 min" | strip: same line + "Run now ≈ +2.1 pp ⏎" · "Run off-hours O" | Waiting on: **Governor** · live cap 3/3 · 2nd in line · ≈14:20 |
| 12 | queued · governor · not_started | "ui-1" + "2 min", not-started icon | counts in queued | badge Not started yet · "live cap 3/3 · ≈14:20" · "2 min" | strip: "Not started yet · live cap 3/3 · ≈14:20" + "Start now ≈ +1.0 pp ⏎" | Status: Not started yet · Waiting on: Governor |
| 13 | queued · governor · paused | "deps-1", paused icon + "1 h 5 min" | counts in queued | badge Paused · "by you" / "Pause all" · "1 h 5 min" | strip: "Paused · by you · Resume p" / "Paused · Pause all · Resume all ⇧P" | Status: Paused · Waiting on: you (Resume) |
| 14 | idle · — · idle | status icon | not counted in the dot | badge Idle, no reason | no strip | Status: Idle |
| 15 | idle · — · suspended | suspended icon | — | badge Suspended | strip: "Suspended to save memory · resumes on demand" | Status: Suspended |
| 16 | done · — · done | done icon, muted row | — | badge Done | no strip | Status: Done · Task: T-4 done |
| 17 | error · — · error | error icon + "2 min" | dot error (highest) | badge Error · "API error 529 (overloaded)" · "2 min" | strip: error line + "Resume r" · "Open in Inbox o" | Status: Error · the line · since |
| 18 | error · startup · error | as 17 | as 17 | badge Error · "no session after 30 s" · "1 min" | strip: startup card (P4.0): "Restart fresh" · "Dismiss" · "Open pane" | Waiting on: **startup** · decision link |

**Rail dot rule**: the project's dot takes the highest level among its agents, in the order error > needs_you > running > waiting > queued (idle and done show no dot). The number next to it is `needs_you` only, hidden at 0. The dot has a shape per level as well as a color (principle 8: never color alone).

**Sidebar run label**: the phase label (§5) from the run record ("Executing"). Only the run_phase outcome `text` writes phases in lower case. The feature badge is `counts.needs_you` of the run.

### 6.1 Status bar

| State | Copy |
|---|---|
| normal | "▶ 4 live / cap 3" · "⧗ 2 queued" |
| nothing live | "▶ 0 live / cap 3" · "⧗ 0 queued" (shown, not hidden) |
| over cap (interactive work above it) | "▶ 4 live / cap 3": the live figure uses the warning color, tooltip "Interactive sessions run above the cap" |
| memory pressure | "▶ 2 live / cap 2 (memory)" |
| paused all | "⏸ Paused all" before the figures |

---

## 7. States matrix: Inbox detail

| # | Case | Marker | Blocks | Actions |
|---|---|---|---|---|
| 1 | permission · open | "Waiting for you · 2 min" | tool_call | Allow once a · Deny r · Deny with message ⏎ · Always allow · Answer in the pane · Open pane o |
| 2 | permission · parked | "Parked · back-1 stopped; resumes on your answer" | tool_call | as 1, without Answer in the pane |
| 3 | question · open (1 question) | open | markdown · options · last_message | Answer ⏎ · Dismiss r · Open pane o |
| 4 | question · open (3 questions, one multi) | open | markdown · options ×3 ("Question 2 of 3 · Pick any") | Answer ⏎ (enabled when every question has a choice or text) |
| 5 | gate spec · open | open | markdown · docs | Approve spec a · Send back r · Send back with notes ⏎ · Open pane o |
| 6 | gate pr · open | open | markdown · diff_summary | Approve: push + open PR a · Reject r · Reply ⏎ |
| 7 | gate restart · open | open | markdown | Approve a · Reject r · Open o |
| 8 | startup · open | open | error · pty_tail | Restart fresh · Dismiss · Open pane o |
| 9 | startup · fresh restart failed | open | error ("restarted fresh, failed again") · pty_tail | adds Delete |
| 10 | ask · open | open | markdown · cost · countdown | Run now a · Keep queued r · Open o |
| 11 | budget · open | open | markdown · cost · countdown? | Apply a · Skip r |
| 12 | trust · open | open | pty_tail | Open pane o |
| 13 | wait · open | open | last_message · pty_tail | Open pane o |
| 14 | error / alert · open | open | error · pty_tail, or markdown | Open o · Dismiss · Restart fresh (spawn, startup) |
| 15 | notion · open | open | markdown (source = page link) | Reply · Dismiss r · Open in Notion ↗ |
| 16 | decided → sent | "Allowing once · Undo u" → "Sent to back-1 · 14:02" | unchanged, actions disabled | none |
| 17 | sent → effect | "Allowed · back-1 resumed · 14:02"; the card leaves the open list at once (`closed` in the `now` event) and `now_snapshot` keeps it 10 min | unchanged | none |
| 18 | PR gate sent → effect | "Pushing orch/checkout/integration…" → "PR opened · checkout #12 ↗ · 15:30" | unchanged | Open PR o |
| 19 | failed | "Failed · push rejected: non-fast-forward" (error color) | unchanged | the kind's actions again (retry by deciding again) · Dismiss |
| 20 | stale | "Stale · the agent moved on" (muted) | unchanged, muted | Dismiss |
| 21 | digest · empty | "Nothing done yet today." | — | — |
| 22 | digest · 5 outcomes | header "Today · 5 outcomes · Clear c" | rows: task_done, merge_done, run_phase, pr_opened, turn_done | Clear c · o on a row opens its subject |
| 23 | digest · cleared | "All caught up · 5 done today" | — | — |

---

## 8. Fixture for the cross-surface test (spec §6 · 1)

Live cap 4 (Green). 1 permission (back-1), 1 question (front-1), 1 queued (back-2, "live cap 4/4"), 1 not started (ui-1), 2 running (lead-1, ux-1). Every surface must show:
- Title badge "2" (decisions)
- Sidebar "2 need you" on checkout
- Rail "2" on shop-api with the needs_you dot
- Overview footer "2 need you · 2 running · 0 waiting · 2 queued · 0 error · 4 live / cap 4"
- Status bar "▶ 4 live / cap 4 · ⧗ 2 queued"
- Inbox "Blocking 2"

The 4 live are back-1, front-1, lead-1 and ux-1: needs_you agents keep their process.

---

## 9. Components for `ui` (ux-brief §3) and what each must cover

1. **Palettes as tokens**: agent status (add **not_started**, F5), attention level (7: needs_you, running, waiting, queued, idle, done, error, each with a dot shape), run phase (11, labels in §5), decision kind (permission, question, gate·spec, gate·pr, gate·restart, startup, ask, budget, trust, wait, error/alert, notion; labels in §3.2), decision lifecycle (open, parked, sent, effect, failed, stale; dismissed and interrupted reuse muted). No palette borrows another's color or icon.
2. **Waiting-on chip**: an icon per `waiting_on.kind` (user, agent, task, governor, limit, startup, external) plus running's activity. Compact and full forms, and overflow, per §1.1. Variants: every row of §6.
3. **Context blocks**: the 10 blocks of §3.3, each with its collapsed/expanded and empty/loading/error states (diff patch).
4. **Lifecycle marker**: the 10 states of §3.4.
5. **Outcome row**: 6 kinds (+2 if F4 is accepted). The digest header in 3 states (§5).
6. **Count labels**: §1.2, including singular/plural.
7. **Agent strip** variants: rows 1–18 of §6 that have a strip.
