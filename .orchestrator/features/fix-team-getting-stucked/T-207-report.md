# T-207 · Supervise the Lead and waits, delivery invariant, `delivery` decision, run report

Branch `orch/fix-team-getting-stucked/back-1`. Covers spec §3.7 and §3.9 and the Rust side of api-contract (all sections). The contract is final and was updated in the same change; `front` has been told.

## What was built

### Blocker vocabulary, in one place (`blocker.rs`, new)
- `blocker::of(&Facts)` returns the string (api-contract §1); `hold_of(&QueueReason)` maps the Governor's reasons; `label()` and `kind()` give the UI label and the cause without its detail.
- It is used by the attention record (`undelivered`), the invariant, the timeline `delivery.blocker`, the `delivery` card copy, the Lead notes and the report (`redelivered` keyed by `kind(why)`).
- Order: `crashed`, `parked`, Governor hold (`governor:<reason>` or `slot`), `starting_up`, `no_ack`, `user_typing`, then `not_idle:<state>`.
- **Added value `unknown`** (idle, live, nothing in its way, no turn made). Contract updated. `not_idle:idle` is labelled "idle" (a wait on an idle Lead).
- Governor mapping: LiveCap / MemoryPressure / BehindQueue → `slot`; OffHours / ScheduledOffHours → `off_hours`; FiveHourGuard → `five_hour`; PausedAll / PausedAgent → `paused`; ModelWindow not exhausted → `model_switch`; the rest (RedTier, limits, HardCap, deferred, exhausted window) → `limit`.

### `delivery` decision kind (`decision.rs`, `stuck.rs`, `orch-app/core.rs`)
- Id `delivery-<n>`, level 2, payload as api-contract §2 plus `turns` and `key`.
- **Action ids:** `open` (key `o`, same as every other card), `retry`, `restart_fresh`, `dismiss`. The contract said `"o"`; I kept the existing id and fixed the contract.
- `inbox_act("delivery-<n>", …)` (the contract's `decision_act`): `retry` → `Runtime::retry_delivery`, `restart_fresh` → FR-415 path, then the item closes; `dismiss` → normal dismissal (messages stay `failed`).
- `retry_delivery`: failed turns of the item go back to `Queued` with attempts 0, its messages are un-failed and requeued (new `Store::retry_messages`, only for messages not seen yet). An item with nothing to resend queues a Continue nudge. The item closes `decided_by: user`, effect `retried`.
- **Auto-close** (`close_delivered`, first step of `team_tick`): every listed message seen or read → `decided_by: core`, `delivered`. An item with no message closes as `recovered` when the agent's next hook event is newer than the item.
- **One decision per episode:** `raise_delivery` dedupes on the episode `key`, against the store (open or closed) and an in-memory set, so a dismissed one is not raised again. Keys: `<agent>:delivery:<first message id>`, `<agent>:stuck_lead:<id>`, `<agent>:stuck_wait:<since>`, `<agent>:working_silent:<turn start>`.
- Each decision for a member also sends its **Lead one note** (a `status` message, a message turn, so the Governor admits it at the user tier and the pump keeps it ahead of Task turns, T-206). The Lead's own decisions go to the user only.

### Hook points
- T-204 `on_delivery_failed`: raises the `delivery` decision (blocker `no_ack`, listed messages and failed turns, PTY tail), then emits `delivery_failed {…, decision: "delivery-<n>"}`.
- T-206 `on_repeated_failure`: raises a `delivery` decision (blocker `not_idle:error`, no messages) next to the existing `continue_failed` event.
- `startup` with `failed: "crash"` was already in from T-206 (card, attention, actions); nothing more was needed.
- `delivery_event` now adds `agent` to every delivery event detail (contract §3).

### Supervision (§3.7)
- **Delivery invariant** (`delivery_invariant`, a `tick_step` per run, after `supervise`): for each non-`Working` agent, any unread, not failed message older than 10 min is a breach. Skipped when the agent already has an open decision (any stored item but `info`, a trust dialog, a pane prompt or an error card: "a decision naming the cause") and for `governor:paused` (the user's own choice).
  - The Lead's breach is episode `stuck_lead`, so the Lead's **idle with unread messages or an unread member report** is the same rule (a report is a message to it).
  - A `stuck` event is emitted for each.
- **Waits on an agent** (`supervise`): a member whose attention wait is on the Lead for 15 min while the Lead is `Idle` is added to the Lead's status note (once per episode, the existing `supervised` set). At 25 min (15 + 10) a `delivery` decision, episode `stuck_wait`, goes to the user.
- **`Working` watchdog** (`working_silent`): when a team agent is silent 15 min, the transcript is read. If the turn ended, the missing Stop is applied (`late_stop`) and `recovered {from_state: "working_silent", how: "late_hook"}` is emitted. Otherwise a `delivery` decision (episode `working_silent`, blocker `not_idle:working`) goes to the user and a note to the Lead. A solo agent keeps the old alert item. The Lead's silence is a decision for the user, with no note.
- **Lead notes at the user tier / exempt from F5:** they are message turns, which T-206 admits at the user tier and picks before Task and Continue turns. I added no new code path for it.

### Attention (§4)
- `AgentAttention.undelivered {count, oldest_ms, blocker}`: unread, not yet seen messages for the agent, older than 60 s. Absent otherwise.
- New inputs `unacked` (a Delivering turn) and `typing` (the user typed in the last 2 s), gathered by `Runtime::delivery_flags`.
- `WaitKind::Delivery`: an open `delivery` item is the agent's top reason at the error rule (level `error`, status `error`, wait ref = `delivery-<n>`), like a watchdog alert. Reason: "2 messages not delivered · paste not acknowledged" or "looks stuck · busy (working)".

### Team view (§3)
- The `message` event detail now has `ids`. `run_json`'s timeline adds `delivery {state, attempts, via, blocker, seen_at_ms, read_at_ms}` to those events (least advanced message of the event). The new kinds (`redelivered`, `delivery_failed`, `recovered`, `stuck`, `tick_panic`) are plain run events with the contract's detail.

### Run report (§5)
- `orch-store/report.rs`: `RunReport.delivery` (`sent/seen/read/failed`, `latency_ms {p50,p95}` by nearest rank, `redelivered`, `recovered`, `stuck`, `tick_panics`). `orchd report` prints two new lines; `--json` carries the section.

## Tests (all new, all pass)
- `blocker.rs`: rule order, Governor mapping, labels.
- `team.rs`: `the_invariant_raises_one_decision_per_episode_and_closes_when_seen` (young message, working target, Lead vs member episode, Lead note, dedupe after dismissal, auto-close), `a_stuck_agent_with_an_open_decision_gets_no_second_one`, `retry_puts_failed_messages_and_turns_back` (also the `on_delivery_failed` hook and its event), `a_silent_working_team_agent_raises_a_decision_and_tells_the_lead`, `a_member_waiting_on_an_idle_lead_is_told_then_escalated`.
- `attention/tests.rs`: `undelivered_…` (60 s rule, blocker, seen, other agent), `an_open_delivery_decision_is_the_agents_wait`.
- `decision/tests.rs`: the card (kind, level, actions, context).
- `orch-store/report.rs`: the Delivery section over a fixture, and the render.
- Docs: SPEC.md FR-407 and FR-411 updated; api-contract.md updated.

## Known limits / follow-ups
- **Late-hook branch of the Working watchdog** has no end-to-end test: it needs a live process (the watchdog skips agents with no PTY) and a transcript file. `working_silent` is tested for the decision path; the transcript path reuses `turn_ended` and `apply_hook` from T-204's paste watchdog.
- **Overwatch (FR-1309)** is not in the Rust code yet, so "the Overwatch gets the same signal" has no receiver. The `stuck` events and decisions are what it would read.
- The invariant uses `created_at_ms` for age, and "unread" (not "unseen"): a message seen and not read while the target is idle is expected to be settled or requeued at turn end (T-204).
- Dismissing a `delivery` decision leaves its messages `failed` with no more attempts (as the contract says); the same episode is not raised again, but later messages open a new one.
- Sandbox: PTY and socket tests can't run here (same set as T-204/T-206). Please run `cargo test --workspace` outside the sandbox.
