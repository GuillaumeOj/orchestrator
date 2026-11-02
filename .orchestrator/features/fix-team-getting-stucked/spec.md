# Fix Team Getting Stuck · Feature spec

> Feature `fix-team-getting-stucked` · Integration branch `orch/fix-team-getting-stucked/integration` · Base `main` (54cdda4)
> Companions: [decisions.md](decisions.md) (one session per agent vs Lead fan-out: pros, cons, industry, **decided: keep A**) · [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md)
> SPEC: FR-203, FR-206, FR-403, FR-407, FR-411, FR-415, §6.4

## 1. Goal and exit criterion

A message, a nudge, a report or a user answer sent to a team agent **reaches the model or comes back as a visible failure, never silently stalls.** Every way an agent can wedge today (decisions.md §2, F1–F10) gets a recovery that runs without an app restart.

**Exit criteria**
1. A chaos test suite (§3.8) reproduces each failure mode F1–F9 with the fake CLI and passes. Each test shows the message reached the model, or a `delivery` decision was raised, within the stated bound.
2. Invariant, checked by the supervisor every tick: **no message stays unread for more than 10 min while its target is not working on a turn**, without a decision or a Lead note naming the cause.
3. The run report counts deliveries, redeliveries, delivery failures and recoveries per cause, so the next real team run can be compared.

Out of scope: a Lead fan-out mode, Claude Code agent teams (decisions.md §4–5), new screens, and changes to the team graph or tool set beyond what is listed here.

## 2. The model after this feature

A message has an explicit lifecycle in the store. Each step is set only by evidence from the CLI, never by the core having *tried*:

```
queued ──paste/inject──▶ handed ──UserPromptSubmit / hook reply written──▶ seen ──turn ends, fully rendered──▶ read
   ▲                        │ no ack in 20 s / agent exits / turn cancelled
   └──────── redeliver ◀────┘  (attempts += 1; after 3 → failed + decision)
```

- `delivered_at_ms` keeps its meaning (*seen*) for existing readers. New columns: `handed_at_ms`, `attempts`, `via` (`turn | inject`), `turn_id`, `failed_at_ms`, `last_error`.
- **Who acknowledges.** A paste is acknowledged when UserPromptSubmit carries its `[t:…]` marker. An injection is acknowledged when the core has written the hook reply within the hook's deadline. The core records the write result (§3.3).
- **Read means the model saw all of it.** A message cut by a render limit is not marked read. Its full text goes to the agent's `messages/` file, and the paste names that file (§3.4).

## 3. Scope

### 3.1 Paste watchdog: no unacknowledged paste wedges an agent (F1) — M
- A `Delivering` turn with no UserPromptSubmit for its marker within **20 s** (`delivery.ack_secs`) is checked. The check reads the ANSI-stripped PTY tail and the transcript.
  - **Marker in the transcript:** the hook was lost. Apply it as a late `TurnStarted`, then continue as normal.
  - **Pasted text still visible in the input box** (Enter swallowed): send `\r` once more and wait another 20 s.
  - **Otherwise** (a menu or modal ate the keys): send **Esc** once, only when the tracker is Idle and the user hasn't typed for 2 s. Then put the turn back to `Queued` with its messages handed back (`attempts += 1`), and restore `ready_at`.
- After **3 attempts** the turn is `Failed`. Its messages become `failed`, and a `delivery` decision is raised (api-contract §2) with the PTY tail. The agent's Lead (or the user, for the Lead itself) gets one note. The agent is unblocked for its other turns.
- **No duplicate:** before a re-paste, the core checks the transcript for the marker (reuse `reconcile.rs` marker matching).
- `ready_at_ms = 0` no longer means "never ready". The watchdog always ends a `Delivering` state.
- A `Delivering` agent becomes suspendable again once its turn is re-queued or failed, never while the paste is in flight (FR-203 stays).

### 3.2 Every live state can receive (F2) — M
- **`StopFailure` with a non-rate-limit error** (process alive): the state becomes `Idle` with `last_error` kept on the record. The core queues one `Continue` turn (`ORCH: your last turn failed (<error>). Carry on, or report where you are.`) unless one is already queued. A second StopFailure on that Continue turn raises a decision instead of looping.
- **`BlockedByLimit`:** the state goes back to `Idle` when the Governor releases the agent (the limit's reset passed or the plan reading is under 100 %). Turns waiting for it are then pasted as usual.
- **Stop with background tasks:** the agent is `Idle` with `background_tasks = n`, no longer `Working`. Only Task and Continue turns wait on background tasks. **Message and User turns are delivered** (Claude Code queues input while background shells run). The watchdog no longer skips these agents.
- Messages already carried by a turn that cannot be pasted are **not** held back from tool-call injection. The "already carried" exclusion applies only while that turn is `Delivering`.

### 3.3 Hook replies and lost events (F4, F8) — M
- **Stop is no longer blocked for messages** (the FR-403 Stop block is removed). At TurnEnded, messages that were injected but not yet seen by a finished turn are handed back (`queued`) and go out as a normal message turn. This removes the race where the core believed in a block the CLI never got.
- **The core applies the state change before it replies** to any hook. A reply that misses the deadline can then no longer leave the tracker inconsistent.
- **Injection is acknowledged by the write.** `inject_messages` marks messages `handed`. They become `seen` only when `orch-hook` reports the reply was written in time: a new short `ack` call, or the next hook of the same session carrying the injected ids. Otherwise they go back to `queued`.
- **No injection into subagents:** a PostToolUse whose payload carries a subagent `agent_id` gets no messages. They wait for the main agent's next tool call or turn.
- **The spool is replayed live:** spooled hook events are applied as soon as the core accepts a connection again (each `orch-hook` call checks its session's spool, and housekeeping sweeps every 5 s), not only at boot. Order is kept per session; late events for a turn that already ended are ignored.
- The list of injected messages moves from memory to the store (`via = inject`, `turn_id` = the running turn's prompt id), so a core restart re-queues injected messages that were never seen.

### 3.4 Read only when fully seen (F7) — M
- A message that doesn't fit the render budget is written in full to `<agent_dir>/messages/msg-<id>.md`. The paste or injection shows its header, its first 600 chars and `full text: <path>` (already readable, FR-206).
- At TurnEnded, a message is marked read when it was rendered in full, or when its file was referenced and the agent read that file (a Read of the path) or called `inbox`. Otherwise it is handed back once. On the second time it is marked read with `truncated: true` in the event, so it never loops.
- `inbox()` returns full bodies or file paths, never "(more…)".

### 3.5 Crashed agents restart; no head-of-line block (F3, F5, F6) — M
- **Process gone** (`Error` after exit, `Killed` not requested by the user): the pump treats it like `Suspended` when it has queued work. It resumes `--resume` through the Governor, at most **2 automatic restarts per agent per hour**. The third crash raises a `startup`-kind decision with `failed: "crash"` (reuse FR-415's restart-fresh path). `resume_agent` works on such an agent without a prior `resume_failed`.
- **Priority in the pump:** per agent, the first deliverable turn wins in the order **User > Message/nudge > Continue > Task**, not the oldest. A turn held by the Governor (off-hours, 5 h guard, slot, model switch) no longer blocks turns of a higher class behind it. Order within a class is kept. Message turns of a team run are admitted at the **user tier** by the Governor (they carry the team's coordination, not new work).
- **Slot deadlock:** a member that has unread messages waiting more than 2 min for a slot frees one. The core suspends the least recently active *live idle* member of the same run that has no unread messages, if there is one. The Lead's own slot (FR-203) is unchanged.

### 3.6 The team tick can't stop delivery (F9) — M
- `team_tick` runs each run's steps (`deliver_messages`, `dispatch`, `supervise`, `watchdog`) under `catch_unwind`. A panic is logged as an event `tick_panic {run, step, message}`, and the other runs and steps carry on. The `teaming` guard is always released.
- Task-backend sync (GitHub/Linear calls) moves to its own thread, off the delivery path, with its own reentrancy guard.
- `rotate_for_task` / `task_boundary` no longer wait up to 5 s for a suspend on the pump thread. The suspend is requested, and the pump moves on.

### 3.7 Stuck detection that sees everything (F10) — M
- **The Lead is supervised too:** idle 10 min with unread messages, a member's report unread, or `Working` with no hook event for 15 min. Its alerts go to the **user** as a decision (`delivery` / `stuck`), since nobody above it is in the team. The Overwatch gets the same signal when it runs (FR-1309).
- **Waits on an agent count:** a member waiting on the Lead (unanswered review request or report) for 15 min while the Lead is idle is a stuck episode, reported to the Lead (and to the user if the Lead doesn't act within 10 min).
- **The delivery invariant** (exit criterion 2) is checked every tick. A breach raises one `delivery` decision per message episode, naming the cause from the blocker list (api-contract §1): `not_idle:<state>`, `starting_up`, `no_ack`, `governor:<reason>`, `slot`, `parked`, `crashed`, `user_typing`.
- **The `Working` watchdog** (no hook event for 15 min, FR-407) now checks the transcript too. If the turn ended there, it applies the missing Stop (the F4 tail case). Otherwise it alerts the Lead as well as the user.
- Lead notes about a member are sent at the **user tier** and are exempt from F5 blocking.

### 3.8 A fake CLI that misbehaves (tests) — M
`tests/fake_claude.sh` gains switches, set per test through the environment:
- swallow the next Enter;
- show a modal that eats keys until Esc;
- drop or delay UserPromptSubmit / Stop hook replies past the deadline;
- emit `StopFailure` (`rate_limit`, `server_error`);
- Stop with `background_tasks`;
- exit mid-turn;
- delay a PostToolUse reply.

One e2e test per failure mode F1–F9 checks the bound in §3.1–3.7. A unit test checks that the priority order and the read rule hold. The existing "a pasted turn is never suspended" test is updated to "…until it is re-queued or failed".

### 3.9 Measurement — S
- Events: `handed`, `seen`, `redelivered {attempt, why}`, `delivery_failed`, `recovered {from_state, how}`, `tick_panic`, `stuck {kind}`.
- The run report (`ReportPane`, `orchd report`) adds a **Delivery** section with these columns:
  - messages sent / seen / read;
  - p50 / p95 time from send to seen;
  - redeliveries by cause;
  - failures;
  - recoveries by state;
  - stuck episodes by kind.

### 3.10 UI — S (no new screen, see ux-brief.md)
- **Feature pane timeline:** message events show their delivery state (queued · handed · seen · read · redelivered n · failed).
- **Agent pane strip and mission control member chip:** "n undelivered · <blocker>" when an agent has messages waiting more than 1 min.
- **New `delivery` decision card** with the PTY tail and the actions *Open pane · Retry now · Restart fresh · Dismiss*.

## 4. Ownership and order

| # | Task | Node | Depends on | Files (mostly) |
|---|---|---|---|---|
| 1 | Message lifecycle columns, ack by marker, paste watchdog, read-when-fully-seen, injection ack, no subagent injection, Stop block removed, live spool replay (§2, 3.1, 3.3, 3.4) | back | — | `orch-store/{migrations,teams}.rs`, `agent-runtime/{runtime,team,delivery}.rs`, `orch-hook`, `orch-protocol/hook.rs` |
| 2 | Wedged states: StopFailure / limit / background, crash restart, pump priority, slot release, tick isolation, backend sync thread (§3.2, 3.5, 3.6) | back | 1 | `agent-runtime/{state,runtime,resources,team,startup}.rs`, `governor/queue.rs` |
| 3 | Supervision of the Lead and waits, delivery invariant, Working watchdog via transcript, events and run report Delivery section (§3.7, 3.9), `delivery` decision kind (api-contract) | back | 2 | `agent-runtime/{supervise,attention,report}.rs`, `orch-app/src/inbox.rs` |
| 4 | Chaos fake CLI and the e2e suite F1–F9 (§3.8) | back | 3 | `agent-runtime/tests/*` |
| 5 | UI: timeline delivery states, undelivered chip, `delivery` card, report section (§3.10) | front (reviewed by ui) | 3 | `ui/src/**` |

The back tasks are sequential because they touch the same runtime files. Front starts once task 3 has landed the contract. Tasks 1 and 2 run with `plan_first` and Opus, since they change the core delivery loop. No UX/UI design pass: small changes inside existing components (ux-brief.md); `ui` reviews front's task.

## 5. Risks
- **Esc on a mis-detected modal** could cancel a turn the CLI did start. Mitigation: Esc only when the tracker is Idle, the transcript lacks the marker, and the user is quiet. At most once per attempt.
- **Removing the Stop block** gives a report that lands mid-turn one extra round trip (a new turn) instead of an in-turn reminder. That is accepted for correctness.
- **Delivering Message turns while background tasks run** relies on Claude Code queueing input (§6.4: UserPromptSubmit fires for a queued message). This is covered by a contract test on the pinned version.
- Hook payload fields used here (subagent `agent_id` on PostToolUse) are confirmed against the pinned Claude Code version in task 1. If one is missing, the fallback is to skip injection when a SubagentStart is open for the session.
