# T-206 · Wedged states, crash restart, pump priority, tick isolation

Branch `orch/fix-team-getting-stucked/back-1`. This covers spec §3.2, §3.5 and §3.6 (decisions F2, F3, F5, F6, F9). The plan was approved with the Pause-all exception. Lead's notes 2 to 4 are covered below.

The branch also brings in my T-204 `/simplify` commit **c2439b9**. It landed after T-204 was merged, so it reaches integration with this merge. It is a refactor only: shared `last_lines`, `cut()`, narrower visibility, and the dead `as_str` removed.

## What was built

### §3.2 Every live state can receive (F2)

**StopFailure that is not a rate limit**
- `state.rs`: the agent becomes `Idle` instead of `Error`, and `last_error` is kept.
- `runtime.rs on_effect(Failed)` now ends the turn the same way a Stop does, through the new `end_turn`. Delivered turns become Done, messages are settled, `ready_at` is set and the Governor is told. Before this change a failed turn stayed `Delivered` forever.
- `continue_after_failure` then queues one Continue turn: `ORCH: your last turn failed (<error>). Carry on, or report where you are.`, flagged with the new `TurnRow.after_failure` (serde default).
  - It queues nothing if a Continue turn is already queued.
  - If the turn that failed was itself an `after_failure` turn, it queues nothing and calls `on_repeated_failure` instead. That is the **T3 hook point**; for now it only emits `continue_failed`.
  - It emits `recovered{from_state:"error", how:"continue"}`.
- The `api_error` alert item is kept as before.

**BlockedByLimit goes back to Idle**
- On a Governor release, `handle_releases → limit_lifted(id, "release")` sets the agent Idle with `ready_at = now` and emits `recovered{from_state:"blocked_by_limit", how}`.
- Some agents never get a release: interactive ones, shadow mode, or `auto_continue` allowed, because the Governor holds no entry for them. For those, housekeeping calls `lift_limits`, which uses the new `Governor::limit_in_force(agent, now)` (a plan window hit, or the agent's model window exhausted). It only acts at least 60 s (`LIMIT_GRACE_MS`) after the agent's last hook, so a limit the Governor couldn't place (an unknown model) can't bounce every 5 s. Emits `how:"limit_over"`.
- `Governor::observe` clears a window's limit when a newer reading is under 100 %, so "the plan reading is under 100 %" counts as the limit being over.

**Stop with background tasks**
- The agent is `Idle` with `background_tasks = n`, and the effect is still `TurnEndedWithBackground`, which now also sets `ready_at`.
- `GateInputs.waits_on_background` is true only for Task and Continue turns. Message and User turns go out.
- SessionStart resets `background_tasks`.
- The FR-407 `watchdog_check` no longer skips agents with `background_tasks > 0`. That count lingers from the last Stop and was hiding silent turns.

**The "already carried" exclusion only applies while Delivering**
- `undelivered(rec, with_queued)`: injection passes `true`, so messages in a Queued turn can be injected. `deliver_messages` passes `false`, so a message is not queued twice.
- After an injection, `drop_carrying_turns` removes the Queued message turns that carried those ids, so the same message is never also pasted. Their other messages are still queued, and the next pass builds a new turn for them.
- If a dropped turn was waiting on a Governor entry and nothing else of that agent is queued, the entry is cancelled. Otherwise its release would start a stray Continue.

### §3.5 Crash restart (F3)
- `startup::crashed(rec)` means: state `Error`, not `resume_failed`, no flag. The pump also requires no live process and no open `startup` item.
- **Killed** is never restarted. Every core kill comes from the user (UI, ctl, project delete), so "Killed not requested by the user" doesn't occur today. A crash already shows up as `Error`, because the stop intent is None.
- **pump_one, branch with no process**: a crashed agent with work is handled like Suspended (admission, slot check, `--resume` spawn).
  - The restart is **counted at the spawn**, as your note 4 asked: `AgentRecord.crash_restarts`, the last hour only. So a resume that dies before SessionStart still uses budget.
  - It emits `recovered{from_state:"crashed", how:"resume"}`.
  - The budget is `CRASH_RESTARTS_PER_HOUR = 2`. The third crash raises **one** `startup` item with `failed:"crash"` ("<agent> keeps crashing", restarts, error, and the PTY tail the core kept at the exit), and nothing is spawned.
- **No double fire with FR-415** (your note 4): `on_exit` calls `check_quick_exit` first for a failed start.
  - Its outcomes are a fresh restart (the agent is live again), `resume_failed` plus an item, or "a resume that has a session", which leaves plain `Error`.
  - Only after that does `crashed()` decide, so the crash path picks up only what FR-415 left alone.
  - FR-415's own fresh restart is not counted as a crash restart.
- **decision.rs**: the startup card for `failed:"crash"` reads "keeps crashing" and offers the same actions (Restart fresh, Dismiss, Delete, Open pane).
- **resume_agent** (the Lead's tool) and **Runtime::resume** (the user's) call `clear_crash`. It resets the budget and closes the open crash item ("resumed"). The Lead's resume then queues its Continue as before. No prior `resume_failed` is needed.

### §3.5 Pump priority (F5) and slot release (F6)

**Priority**
- `delivery::pick_turn` / `by_priority` order an agent's turns **User > Message (nudges are Messages) > Continue > Task**, with queue order kept within a class (a stable sort).
- They skip turns still waiting on a Governor entry, and Task/Continue turns while `background_tasks > 0`.
- The pump resets stale `Waiting` entries for every turn (not only the first), then works on the picked turn.
- The ride-along batch is the agent's other turns in priority order. It stops at a command, a held turn, a turn that waits on background work, or a not-yet-admitted Task (its model switch, FR-819, happens when it is picked itself).

**Team messages at the user tier**
- `decide_turn` asks with `interactive: true` for a Message turn of a team agent.
- It doesn't while Pause all is on, the agent is paused (a run pause pauses its agents), or a plan limit is in force. Without that last condition, a limited agent would re-hit the limit with every message.

**Slot release**
- The pure rule is `resources::slot_victim`.
- `Runtime::free_slot_for_messages` runs in housekeeping when no idle or pressure suspend already freed a slot this pass.
- It applies when the counted live agents fill the cap and a team member has been held on a slot for 2 min or more, counted from its oldest unread message. "Held on a slot" means a Governor entry waiting on `MemoryPressure` or `LiveCap` and a queued turn.
- It suspends the least recently active live, suspendable, non-reserved member of the same run that has no unread messages. Emits `slot_freed{for, suspended}`.

### §3.6 Tick isolation (F9)

**team_tick**
- Each run's steps (context, end_run, dispatch, supervise, deliver_messages, progress) run through `tick_step`, which uses `catch_unwind`.
- A panic becomes `tick_panic {run, step, message}` on the run's timeline (the global events for `run:"-"`), and the other steps and runs carry on.
- In housekeeping, the team thread wraps `team_tick` and `watchdog` the same way, so the `teaming` guard is always released.

**Task-backend sync** moved to its own thread with its own `syncing` guard. It doesn't use up its 5-min slot while a previous sync is still running.

**rotate_for_task / task_boundary** use the new `suspend_in_background`.
- It sets the stop intent right away, spawns the suspend, and treats a second request as a no-op.
- If the suspend fails, the stop intent is put back and `suspend_failed` is emitted.
- The pump **pastes nothing into a live agent whose stop is under way**. The task turn queued right after a rotation goes to the new session's process.

### Other
- **create_task** (your note 2): the body is stored whole. Above 64,000 chars it is refused with "put the details in a feature doc and name it in the body". It is never cut.
- **SPEC.md**: FR-203 (slot deadlock, crashed agents use slots, suspends that are not waited for), FR-403 (live states, priority, user tier, the injection exclusion, tick isolation, the `recovered` values) and FR-415 (crash restarts) are updated.

## Tests

**New**
- `state.rs`
  - StopFailure `server_error` → Idle with `last_error` kept.
  - Stop with background → Idle.
  - SessionStart resets the background count.
- `delivery.rs`
  - `the_pump_takes_the_first_turn_that_can_move_by_class`: user before a held task, FIFO within a class, held turns skipped, background holds Task/Continue only.
  - `gate_rules`: background blocks only `waits_on_background`.
- `resources.rs` `a_member_waiting_for_a_slot_frees_one_from_its_own_run`: same run only, least recently active, never the lead, never one with unread messages or a busy one, nothing under 2 min or with a free slot.
- `runtime.rs`
  - `a_failed_turn_leaves_the_agent_idle_with_one_continue_and_no_loop`
  - `a_limit_released_or_over_gives_the_agent_back`: grace, a limit in force, a release, no limit.
  - `a_crashed_agent_with_work_is_resumed_twice_an_hour_then_waits`: Killed is left alone, a failed start is left to FR-415, a crash is resumed, the budget is used then the item raised once, `clear_crash`, and the hour window.
- `team.rs`
  - `a_panic_in_one_step_never_stops_delivery`: a test-only, thread-local panic injected in `dispatch`.
  - `a_message_in_a_queued_turn_is_injected_but_not_one_being_pasted`
  - `a_task_body_is_stored_whole_or_refused`
- `governor/tests/rules.rs` `a_limit_is_lifted_at_its_reset_or_by_a_newer_reading_under_100`

**Updated**
- `the_watchdog_flags_long_and_silent_turns`: background no longer hides a silent turn.
- `pacing::a_task_model_switches_at_the_task_boundary_and_back_at_the_next_task`: `task_boundary` must return within 1 s, the switch is asked once, and the test waits for the background stop.
- e2e "a pasted turn is never suspended" was already renamed to "…until re-queued or failed" in T-204. I re-read it and the FR-415 e2e test against the new rules, and both still hold: `resume_failed` and the open item keep the crash path away from FR-415's cases.

**Results in my sandbox** (with `ORCH_TEST_TMP` set to a writable folder, because `/tmp` is not writable here)
- `cargo test -p agent-runtime --lib`: every test that doesn't need a PTY or a socket passes. The remaining failures are environment limits, the same set as in T-204:
  - `openpty`: handover watchdog end-to-end tests, pty, procs;
  - socket binding: otlp, `runtime::otlp_requests…`;
  - the worktree watcher;
  - the three `pacing` model-switch tests, which also hardcode `/tmp`.
- `cargo test -p governor`: green.
- clippy `--workspace --all-targets`: clean. fmt: done.
- `cargo test --workspace --no-fail-fast`: every crate is green except for tests blocked by the sandbox:
  - agent-runtime lib: 258 passed, 19 failed (the set above);
  - `e2e_fake_claude`: 10 tests can't bind the core's socket (`Operation not permitted`);
  - `orchd crash_restart`: the core never comes up, for the same reason;
  - orch-app core/terms: 5 tests (PTY and socket);
  - orch-hook: 7 tests that bind the fake core's socket;
  - orch-protocol bus: 3;
  - orch-mcp: 1;
  - orch-status: 1.

  None of these fails on an assertion.

**Please run** `cargo test --workspace`, including `e2e_fake_claude`, outside the sandbox, or allow PTYs and local binding.

## Known limits / follow-ups
- A crash in the middle of a turn leaves that turn `Delivered`. The resumed session doesn't end it, and `quiet_for_task` stays false until a later Stop. Restarting needs queued work, as the spec says; T3's supervision covers an agent that crashed with nothing queued.
- `limit_in_force` treats a model window as a limit only when it is exhausted. When the model is unknown (`other`), only the 60 s grace stops it from flapping.
- `Killed` is never auto-restarted (see above). If a core-initiated kill is ever added, it needs its own stop intent.
