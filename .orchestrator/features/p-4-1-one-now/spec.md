# P4.1 — One "now" · Feature spec

> Feature `p-4-1-one-now` · Integration branch `orch/p-4-1-one-now/integration` · Source: [docs/SPEC.md](../../../docs/SPEC.md) §11 P4.1 (v0.12), FR-1301, FR-1303, §9.1 principles 10–11, §9.5 "Waiting on", §9.6 level 4 · Why: [brief 2026-10-28](../../../docs/brief/2026-10-28-overwatch.md) "No shared now" and "The Inbox throws its structure away", [ADR-011](../../../docs/adr/011-overwatch-and-attention-model.md) §1–2
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (copy, components, states) · [decisions.md](decisions.md)

## 1. Goal and exit criterion

SPEC §11: *"Attention records in the core, pushed on change (FR-1301); Decisions keep their payload and lifecycle (FR-1303); outcome digest; every existing surface renders attention (sidebar, rail, Overview, agent header, Inspector, status bar)."*

**Exit criterion: every "needs you", live and queued count agrees across surfaces; no per-pane polling left.** Checked by (a) one UI test that renders every surface from one fixture and asserts the same numbers, (b) a grep in CI-run tests that no pane or store module has a data `setInterval`/poll loop (the terminal byte pump, debounces, toast timers and the 30 s relative-time clock are not polls), (c) the demo in §6.

This phase changes **no layout**: Mission control (P4.2), the chat and cards (P4.3) come later. The existing panes keep their place and render the core's records instead of their own derivations. The Inbox pane stays until P4.3 and renders the new Decision shape.

## 2. What the code does today (HEAD 44599b6)

| Item | Today |
|---|---|
| Transport | The UI polls `app_state` every **1 s** (`App.svelte:89`), an untyped JSON rebuilt from scratch per call (`core.rs:876`). Seven panes poll their own views: Feature 4 s, Board 5 s, Planner 5 s, Diff 15 s, Team canvas 15 s, Project 60 s, Report 60 s (+ Notion status 60 s). `notify.rs` polls `Core::attention()` every 3 s. The only pushed events are `inbox_outcome`, `menu`, `dev`. |
| Change notification in the runtime | None. `state::apply` (state.rs:131) and ~20 direct writes mutate agent state; nothing signals a change. |
| Agent state in the UI | `displayState()` (`ui/src/lib/status.js:25`) maps raw states + queue entries; each surface adds its own rules. **"Needs you" has four definitions**: Sidebar (inbox kinds gate/question/permission/startup + waiting without item), IconRail and `nextWaiting` (waiting_for_you ∨ trust), Overview (+ waiting_gate), title badge (core, levels 1–3). "Queued" is `queue.length`, `queue_entry ∨ created`, or `snapshot.queued`. Governor text is "Admitted" / "admitted · n/cap live" / "queued 3rd (tier)" / "queued 3 (tier)". Status bar's live count includes archived agents. Feature pane falls back to raw unmapped states. |
| Waiting on | Not modelled. Governor reason + ETA exist (`QueueEntry`, governor/queue.rs:96). Task deps exist (`Task.depends_on`) but nothing marks a blocked task; `orch_team::run::blocked_by` is unused. Agent-on-agent waits exist nowhere. `intervention.rs::wait_of` (P4.0) computes a local cause for pane interventions; `supervise.rs` computes another for FR-411. |
| Inbox items | Store kinds: permission, gate (push_pr, spec_approval, restart_gate), question, startup, alert, info, budget, notion. **Without payload**: alerts at runtime.rs:1388/2015/2139, pacing.rs:229, reconcile.rs:163/180/195/342; budget (runtime.rs:539). **Synthesized, never stored, no payload**: `trust-`, `wait-`, `error-`, `ask-` (Governor ask), `info-<turn>`. Status: Pending, Decided, Expired, Interrupted, Dismissed, Parked; `decided_by` (P4.0). No subject, no sent/effect stage, no thread. |
| Digest | `info-<turn>` = "<agent> finished a turn" with the first 120 chars of the **prompt sent to the agent**; not dismissable; user-typed turns absent. Outcomes exist as run events (`task.done`, `git.merge`, `run.state`) but are not shown; a PR opening has no event with its URL (`pr_opened`, team.rs:1617, sets `run.pr_url` only). |
| Run phase | Stored: `team_runs.status` (`RunState`: draft, spec, spec_gate, planning, executing, integrating, pr_gate, done, paused, deferred, failed), transitions logged as `run.state`. |

## 3. Scope

### 3.1 Attention records — FR-1301 (M)
A new pure module `agent-runtime/src/attention.rs`: `compute(&Inputs, now) -> World` where `Inputs` is the StateFile (agents, inbox, turns, team seats), the Governor queue entries and pause set, the team runs and tasks, and the synthesized waits (trust dialog, pane prompt). No I/O, no locks; unit-tested on fixtures.

**Agent record** (shape in api-contract §1):
- `level`: `needs_you | running | waiting | queued | idle | done | error` — the lane.
- `status`: the §9.5 badge key (`working, waiting_for_you, waiting_gate, queued, not_started, paused, blocked_by_limit, idle, suspended, error, done`), computed here; `displayState()` in the UI goes.
- `waiting_on?: {kind, ref?, since_ms}` with `kind ∈ user | agent | task | governor | limit | startup | external`.
- `reason`: the short text of §9.5's "Waiting on" table, written by the core; patterns in `docs/design/p4-design-notes.md` §2 (D11: never repeats the status label, no duration — the UI appends `since`), e.g. `permission: Bash pnpm add zod`, `→ back-1 · review of T-4`, `live cap 3/4 · 2nd in line · ≈14:20`.
- `activity?`: for `running`, the current tool and its short input, or `writing`, or `starting`.
- `live`, `since_ms` (when this level+wait began), plus identifiers (agent, project, run, node, task).

**Rules, first match wins** (the single definition; `needs_you ⇔ waiting_on.kind = user`):
1. **error** — state `Error`, or an open error decision for the agent (spawn failure, watchdog, API error, startup failed after the fresh retry, startup blocked: decisions D3). `waiting_on.kind = startup` for startup failures, else none; `reason` = the error's one line.
2. **needs_you** — an open decision about this agent that waits on the user: permission (pending or parked), question, gate (spec, PR, restart), Governor ask, trust dialog, or `WaitingForYou` with no item (a prompt only in its pane: `reason` "a prompt in its pane"). `ref` = the decision id (oldest first, ranked permission > question > gate > ask > trust > pane prompt). `since` = the decision's `created_ms`. A run's spec and PR gates are `needs_you` on the Lead.
3. **waiting · limit** — `BlockedByLimit`: `reason` "5h window full · resets 16:00" / "week full · resets Mon 09:00".
4. **queued · governor** — a Governor queue entry, or `Created` with no process (`status: not_started`), or paused (`status: paused`, `reason` "Paused by you" / "Paused · Pause all"). `ref` = agent id (queue key); `reason` = Governor reason + position + tier + ETA in one string; `since` = when the entry was first queued.
5. **waiting · startup** — spawned, no SessionStart yet, under the 30 s startup timeout: `reason` "starting · times out after 30 s" (status working).
6. **running** — `Working`/`Starting` after SessionStart. If the current tool is an MCP tool running for ≥ 5 min: **waiting · external** instead, `reason` "waiting on penpot execute_code".
7. **waiting · agent** — idle with an open task (or a Lead with a non-terminal run) and one of, in order:
   - its task is in **Review** and a review was asked (`request_review`): → the reviewer;
   - it submitted a **plan** (`plan_first`) not yet decided: → the Lead;
   - it sent a message or handoff to a member since its last turn and got no reply: → that member, `reason` "→ back-1 · message";
   - a Lead whose run is `executing`/`integrating` with tasks in progress: → the members working them (`ref` = first; `reason` "→ back-1, front-1 · T-3, T-4").
8. **waiting · task** — a member with no task in progress whose next task assigned to its node has unmet `depends_on`: `reason` "blocked by T-2 (back-1, in progress)" (`orch_team::run::blocked_by`).
9. **done** — a member whose run is `done`, or whose assigned tasks are all Done and none open; an archived agent.
10. **idle** — everything else (solo idle, suspended with no work: `status: suspended`).

`since_ms` comes from a stored timestamp when one exists (decision, queue entry, turn, task, run event); otherwise the publisher remembers when the record's `(level, kind, ref)` first appeared. After a core restart, such remembered times restart at boot (D7).

**Run record**: `{run, feature, project, phase (RunState), phase_since_ms, level, members: [agent ids], blocker?: {agent, waiting_on, reason}, counts}`. `level` = the highest member level in the order error > needs_you > running > waiting > queued > idle > done (a run in `spec_gate`/`pr_gate` is needs_you through its Lead). `blocker` = the first member wait that is not internal to the run (kind user, governor, limit, startup, external) — "what this run waits on from outside".

**Counts** (`World.counts`, global and per project and per run): `needs_you` (agents at needs_you), `decisions` (open decisions waiting on the user, levels 1–3 — the badge), `running`, `waiting`, `queued` (agents at level `queued`, nothing else — an agent with a Governor ask is at needs_you and is not counted here), `idle`, `error`, `live` / `live_cap` (live processes, archived excluded), `done_today` (outcomes today). Every surface shows these numbers and computes none.

**Replaces the local causes**: `intervention.rs::wait_of` (P4.0) and `supervise.rs`'s stuck detection read the agent's attention record instead of their own rules (same event detail shapes; `waited_on` takes the record's kind, with P4.0's finer cause kept in `detail.cause`).

### 3.2 One push channel — FR-1301 (M)
- **Change detection**: runtime mutations (state apply, direct state writes, inbox add/close, turn status, queue refresh, team task/run/message writes, usage sample) set a `dirty` flag. The 200 ms pump recomputes `World` when dirty, and the 30 s tick recomputes regardless (time-based reasons: "≈14:20", "7 min", external ≥ 5 min). Only records that differ from the last published ones are sent.
- **One sink, one event**: the runtime gets a `NowSink` (like P4.0's `OutcomeSink`); the app emits **one Tauri event `now`** with `{rev, attention, decisions, outcomes, topics}` deltas (api-contract §3). `rev` increases by one per event; a UI that sees a gap, or wakes from hidden, calls `now_snapshot` and replaces its copy.
- **Pane views are invalidated, not polled**: the event's `topics` lists what changed — `agents`, `queue`, `usage`, `inbox`, `features`, `feature:<id>`, `board:<project>`, `planner`, `teams`, `project:<id>`, `worktree:<path>` (an agent's Edit/Write/NotebookEdit/Bash PostToolUse in it, a commit, merge or sync), `report`, `integrations`, `memory`, `power`, `banners`. A pane refetches its view when its topic arrives (coalesced, at most one fetch in flight), on mount and on window focus. `app_state` stays as the snapshot command for the shell's non-attention data (projects, usage, memory, setup, banners) and is fetched on the `agents|queue|usage|memory|power|banners|setup` topics — not on a timer.
- `notify.rs` subscribes in-process to the same `World` instead of polling `Core::attention()` every 3 s.
- **ctl**: `CtlRequest::Now` → the `World` snapshot; `orchd now [--json]` prints agents and runs with level, status, reason and since (also the base for P4.4's `world()`).

### 3.3 Decisions — FR-1303 (M)
The Inbox item becomes a **Decision** (shape in api-contract §2), built by one function in the core for every kind.
- **Subject**: `{agent?, role?, node?, project?, feature?, run?, task?}`, filled from the agent's seat and the payload.
- **Every kind has a payload** (none reduced to strings). New payloads at the creation sites: API error `{error, status?}` (runtime.rs:1388), trust `{tail}` (2015), spawn failure `{error, cmd}` (2139), battery `{on_battery, queued}` (pacing.rs:229), reconcile problems `{worktree, problem, files?}` (reconcile.rs:163/180/195/342), budget `{reason, cost_week_pp, cost_5h_pp, default, deadline_ms}` (runtime.rs:539). Synthesized items: `trust` `{tail}`, `wait` (pane prompt) `{tail, last_message?}`, `error` `{error, tail}`, Governor `ask` `{action, cost, default, deadline_ms}`. All alerts go through `add_item` (so all emit `inbox_added`).
- **Context blocks** (typed, ordered; the card renders them in order): `tool_call {tool, input, cwd}` · `markdown {text, source?}` · `docs {files:[{path, lines}]}` · `diff_summary {files:[{path, added, removed}], added, removed, patch: "on_demand"}` · `options {questions:[{question, header?, multi, options:[{label, description?}]}]}` (every question of a multi-question `AskUserQuestion`) · `last_message {text, at_ms}` · `pty_tail {text}` (only when nothing better exists) · `cost {week_pp?, five_hour_pp?, eta_ms?}` · `countdown {default, deadline_ms}`. Kind → blocks in api-contract §2.3. The PR gate's diff summary is computed once when the gate is raised (stored in the payload), the patch fetched on demand (`decision_patch`).
- **Lifecycle** (stored): `open → decided (by user | policy | overwatch | pane | rule:<id> | expired | core) → sent → effect`, or `failed`, or `expired → parked`, or `stale`, or `dismissed`/`interrupted` (existing statuses keep their meaning). New columns: `subject`, `thread`, `sent_at_ms`, `effect`, `effect_at_ms`, `error`. **sent** = the core delivered the answer (hook replied, user turn pasted or queued, PR push started); **effect** = what then happened: `resumed` (the agent started a turn), `queued` (with ETA), `pr_opened` (with URL), `delivered`, `allowed`, `denied`, `restarted`, `dismissed`, `applied`, `skipped`, `deleted`, or `failed: <why>`. Each transition is pushed (`decisions` in the `now` event) and the P4.0 `inbox_outcome` event becomes that transition (kept emitted for one phase, then removed in P4.3).
- **Thread**: `thread` = the run id, else the agent id. A decision lists `thread_prev?` (the previous decision in the same thread) so a follow-up question can sit under the earlier answer (rendered in P4.3; stored now).
- **Stale**: a decision whose context changed (the agent moved on: a newer turn started for a question/permission; a gate whose branch moved) is marked `stale` with why; P4.0's behaviour for permission items is unchanged.
- One migration (10): the new `inbox_items` columns, status values `sent`, `effect`, `failed`, `stale`; old rows read with `subject` null, `thread` = agent id, lifecycle mapped from their status.

### 3.4 Outcome digest — §9.6 level 4 (M)
- Level 4 lists **outcomes, not turns delivered**: `task_done {task, title, by}`, `merge_done {branch, into}` (ok merges only), `pr_opened {url, run}`, `run_phase {from, to}`, `run_done`, and for **solo sessions** only `turn_done {summary}` = the first line of the agent's **last assistant message** (stored from the Stop hook's `last_assistant_message`, 280 chars). Team members' turns are not outcomes; their tasks are.
- Built from run events and a new `run.pr_opened {url}` event written by `pr_opened` (team.rs:1617) and a solo PR opening (inbox.rs `open_pr`, which also writes the URL into its decision's effect).
- `outcomes` table-free: a store query over events of today (local day), newest first, with the outcome id = event id. **Dismissable**: `digest_seen(up_to_event_id)` stores a per-day watermark; seen outcomes leave the Inbox digest but still count in `done_today` (P4.2's Done today lane).
- `info-<turn>` items are removed.

### 3.5 Every surface renders attention (M)
Front, against api-contract §3–4; copy and visuals in ux-brief:
- **One store slice**: `app.now = {rev, agents, runs, decisions, outcomes, counts}` fed by `now_snapshot` and the `now` event. Accessors `attention(agentId)`, `runAttention(runId)`, `counts(scope)`. `displayState`, `notStarted` and every ad-hoc count are deleted; `status.js` keeps labels, colors and icons only.
- **Sidebar**: agent badge = record `status`; feature badge = run counts `needs_you`; run label = phase; dots from level.
- **Icon rail**: project dot/count = project counts `needs_you` (grouped by the record's `project`).
- **Overview**: rows show status + `reason` + since; filter chips and footer totals = counts; the governor column = the record's queue reason (no index arithmetic).
- **Agent pane header and strip**: status, `reason`, since; the strip's text = the record's reason; "Not started yet / Start now" when `status = not_started`.
- **Inspector**: status, waiting-on line, reason (no own "Admitted").
- **Status bar and Banner**: `live / live_cap`, `queued`, from counts.
- **Title badge**: `counts.decisions`.
- **Feature pane**: members from records (no raw-state fallback); run phase from the run record.
- **`nextWaiting` (leader w)**, palette jump: order by `needs_you` records' `since`.
- **Inbox pane**: renders Decisions: subject line, context blocks in order (one component per block type), lifecycle marker (open, parked, sent, effect, failed, stale), actions; level 4 = outcome digest with **Clear** (`digest_seen`). Toasts state the **effect** from the lifecycle push, never before `sent` ("Allowed · back-1 resumed", "PR opened ↗").
- **Polls removed**: the 1 s `app_state` loop, the seven pane `setInterval`s, `flushDecisions`' extra refresh. Kept: the 1 s undo-window flush (a local timer that sends due decisions, not a fetch), the 30 s relative-time clock, the terminal pump, debounces, toasts.

## 4. Out of scope (P4.1)
- Mission control, lanes, run rows with a phase stepper, the per-project filter (P4.2). The Inbox and Overview panes stay.
- The chat column, cards, card threading UI, collapsed lines (P4.3). Decisions store `thread` now; nothing renders threads yet.
- The Overwatch, policy file, `overwatch` decider (P4.4). The `policy` and `overwatch` deciders exist in the enum only.
- The full P4 Penpot gate (§9.0: new shell, Mission control, card storyboard) — before P4.2. P4.1 designs only the pieces it renders: waiting-on chip, level/status/decision-kind/run-phase palettes, context blocks, lifecycle marker, outcome row (ux-brief §3).
- Intake (P4.5).

## 5. Non-functional
- **Performance / memory (NFR-1b)**: `compute` on 30 agents + 10 runs < 1 ms (bench test); the push sends deltas only; the UI does no work per second when nothing changes. Removing the 1 s poll should lower the webview's idle CPU; measure idle CPU and WebContent memory before/after with `docs/app/RUNNING.md`'s method and put the numbers in the PR.
- **Consistency**: the UI never combines two sources for one fact (no "inbox item OR state" rules left).
- **Data (NFR-8)**: migration 10 only; old rows readable; events retention unchanged (30 days).
- **Security (ADR-009)**: no new agent capability; `last_assistant_message` is stored like other event details (blob above the threshold).
- **Tests**: `make ci` green. Core: `attention::compute` table tests for every rule and precedence, run records, counts; publisher sends only changed records and a gapless `rev`; every Decision kind serialises with subject, blocks and lifecycle; lifecycle transitions for permission, question, spec gate, PR gate (effect `pr_opened` with URL), budget ask; digest from run events; migration 10 on a P4.0 database. e2e on the fake claude: a permission prompt goes needs_you → running with one `now` event each. UI: the cross-surface count test (§1), each surface renders `reason` from a record, no `displayState` import left, each context block component, toasts only after `sent`. Tests hold at any date, hour and time zone.

## 6. Acceptance (demo at the PR gate)
1. With 1 permission prompt, 1 question, 1 queued agent, 1 not-started agent and 2 running: sidebar, rail, Overview, status bar, title badge and the Inbox show the same needs-you (2), decisions (2), queued (2) and live numbers.
2. An agent waiting on a review reads "→ lead · review of T-4 · 3 min" in the Overview, its pane header and the Inspector; a member with a blocked task reads "blocked by T-2 (back-1, in progress)".
3. `orchd now` prints the same records.
4. Answer a permission from the Inbox: the card shows sent, then "Allowed · back-1 resumed" when the agent starts its turn; the record moves to running with no refresh.
5. A PR gate card shows the diff summary with per-file line counts; approving it ends with "PR opened ↗ <url>" and the outcome appears in the digest; Clear empties the digest.
6. The webview's network/IPC log over 60 s idle shows no `invoke` call; touching a file in a worktree refreshes an open Diff pane.
7. A multi-question `AskUserQuestion` shows every question with its options.
