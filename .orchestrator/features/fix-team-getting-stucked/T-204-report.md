# T-204 · Message lifecycle, CLI acks, paste watchdog, read when fully seen

Branch `orch/fix-team-getting-stucked/back-1`. This is the full plan as built and the report. The running orchestrator still cuts long messages, so the report lives here.

## What was built (the plan, as implemented)

**Store (orch-store)**
- Migration 13 on `messages` adds `handed_at_ms`, `attempts`, `via` (turn|inject), `turn_id`, `failed_at_ms` and `last_error`. It backfills `handed_at_ms = delivered_at_ms` and adds an index on (to_agent, via, read_at_ms).
- `delivered_at_ms` still means *seen*.
- `Message::delivery_state()` returns queued | handed | seen | read | failed (for T-207's timeline).
- New API:
  - `hand_messages(ids, agent, via, turn_id, at)`
  - `see_messages`
  - `requeue_messages(ids, why, attempt)`
  - `undeliver_messages`, which is now `requeue` without counting an attempt
  - `fail_messages`
  - `injected_unread(agent?)`
  - `messages_by_id`
- `deliver_messages` is removed.

**Protocol and orch-hook**
- `Reply::Hook` gains `ack: Vec<u64>`, `HookMsg` gains `acks`, and there is a new `Msg::HookAck`. All are additive serde fields.
- orch-hook **prints first**, then `confirm()` sends `HookAck` within `ACK_DEADLINE` = 300 ms (connect, read and write all bounded).
- If that call fails, the ids are appended to `<agent dir>/acks.pending`. The next hook takes them (rename first, so a concurrent hook neither sends them twice nor loses them) and sends them in `HookMsg.acks`. They are put back if that hook fails too.
- Only PostToolUse replies carry `ack`, so PreToolUse and PermissionRequest never pay for it.
- `spool::take()` renames the spool before reading it.

**Core (agent-runtime; new module `handover.rs`)**
- **Paste** (`pump_one`): the turn's messages become `handed(via=turn, turn_id=turn)`. The turn records `file` when the paste went to `<turn>.md`.
- **Seen** (`mark_delivered`, on UserPromptSubmit or QueuedMessage markers): the turn's messages become seen. Emits `seen`.
- **Injection** (PostToolUse): the messages become `handed(via=inject, turn_id=prompt_id)` and the reply carries `ack`.
  - `serve_hook` (used by `server.rs` and `orch-app/core.rs`) writes the reply. If the write fails, the messages are re-queued (`write_failed`).
  - `on_hook_ack` marks them seen. It only acts on messages bound to the same agent that are still handed.
  - `HookMsg.acks` is applied first thing in `on_hook`.
- **No injection inside a subagent**: `in_subagent(payload)` checks `agent_id` (CLI 2.1.x sets it only for hooks fired in a subagent). The fallback (`Tracker::subagents`) is documented in that predicate but not enabled.
- **FR-403 Stop block for messages removed.** `stop_check` keeps only the pane-question block, which by design is decided before the state change. Every other hook applies the state change before it replies.
- **TurnEnded** (`settle_messages`):
  - Delivered turns become Done.
  - Injected messages never acked are re-queued (`no_ack`, attempts + 1).
  - The read rule is applied to the ended turns' messages and to this agent's seen-but-unread injected ones.
  - A paste still in Delivering gets its watchdog clock restarted. Otherwise, after a user turn, the "no hook since the paste" check would never pass.
- **Read rule** (`fully_seen`). A message is read when one of these holds:
  - it was shown in full;
  - every file it was written to was referenced in a tool call during the turn (the paste's `<turn>.md`, its own `msg-<id>.md`). This covers `Read` and also `cat`: any PostToolUse input naming `messages/<name>.md`;
  - `inbox` was called.

  Otherwise it is handed back once (`last_error = not_fully_seen`, no attempt counted). The second time it is read and `read{truncated:true}` is emitted.
- **Rendering** (`render_for`):
  - a message over its inline cap (3,000 chars in a turn, 1,200 injected, 6,000 in inbox) is written to `messages/msg-<id>.md` (header + full body) and shown as its first 600 chars and `full text: <path>`;
  - messages past the budget are left out and wait for the next hand-over;
  - **no "(more…)" anywhere**, `inbox()` included.
- **Paste watchdog** (`paste_watchdog`, in the 5 s housekeeping pass on its own guarded thread; `delivery.ack_secs`, default 20 s). The rule is the pure function `next_step`, applied in this order:
  1. The transcript has the turn's marker → late UserPromptSubmit, plus a Stop if the transcript ends with `end_turn`, through `apply_hook(replay)`. Emits `recovered{how:"late_hook"}`.
  2. The input box shows the paste (marker, command text, or "[Pasted text") and Enter was not retried yet → `\r` once, `enter_retried`, the clock restarts. Emits `redelivered{how:"enter"}`.
  3. **Only when** the tracker is Idle, the user has been quiet 2 s **and** no hook came since the paste (and step 1 found no marker) → **Esc once per attempt**. The turn is re-queued with attempts + 1, its messages are re-queued with `no_ack`, and `ready_at = now + 0.5 s`. Emits `redelivered{attempt, why, how:"esc"}`.
  4. Otherwise, wait.
- **After 3 attempts**:
  - the turns become `Failed` (new `TurnStatus`, pruned like Done) and their messages failed;
  - `ready_at` is restored, so the agent's other turns go out;
  - `delivery_failed{turns, messages, attempts, blocker:"no_ack", since_ms, tail}` is emitted (tail: last 40 non-empty lines).
  - **T-207 hook point**: `Runtime::on_delivery_failed(&DeliveryFailure)`.
- **No double paste**: `pump_one` checks the transcript before re-pasting a turn with attempts > 0 (`already_in_transcript`).
- **Live spool replay** (`replay_spool`): runs first on every hook of that agent (reconnect) and in the 5 s sweep. Records are applied in file order through `apply_hook(.., replay=true)`, which gives no answer, no hold, no injection and no Stop block.
  - Stale records (older than the agent's last applied event) are dropped.
  - A stale UserPromptSubmit still marks its turns delivered and their messages seen.
- **Boot**: `recover()` calls `requeue_injected_unseen(None, "restart")`. The in-memory `House.injected` is gone.
- **Events** go to the run timeline for team agents: `handed`, `seen`, `read`, `redelivered`, `delivery_failed`, `recovered`, plus `spool_replayed`.

**Your point 1: the plan was cut (F7)**
- `submit_plan` cut the plan to 6,000 chars in the message to the lead (8,000 in `flow.plan`), and `post_message`/`add_run_message` cut every body to 8,000. None of these cuts left a file or a marker.
- Now plans and messages are stored whole. Above 64,000 chars they are **refused** with "put the details in a file and send its path", never cut.
- The lead gets the first 600 chars and the file path. Test: `team::tests::a_long_plan_reaches_the_lead_whole_never_cut`.
- Not changed: `create_task` still cuts a task body at 8,000 chars (same class, outside this task).

**SPEC.md**: FR-206 and FR-403 are rewritten to describe the above.

## Tests
- **New unit tests** in `handover/tests.rs` cover:
  - the pure rules: `in_subagent`, `referenced_files`, `fully_seen`, and the `next_step` table, including the Esc safety conditions;
  - rendering to files with no "(more";
  - a long message read only after its file is read, else handed back once, then truncated;
  - inbox;
  - subagent and lost-reply handling, `HookMsg.acks`, and another agent's ack;
  - restart re-queue;
  - re-queue and failure at the 3rd attempt (event, tail, other turns unblocked);
  - late hook start and end;
  - the clock restart at turn end;
  - the no-double-paste check;
  - live spool replay with stale events dropped.
- **Plus**: the orch-store lifecycle test, the migration 13 backfill test, the orch-hook ack and acks.pending tests, and the protocol round trip.
- **Updated**:
  - `runtime::tests`: the FR-403 Stop-block test is now "read or handed back at Stop, never blocked"; the read and drop tests follow the new states.
  - e2e: "a pasted turn is never suspended" became "…until re-queued or failed".

**Results in my sandbox**
- `cargo test -p orch-store`: green.
- `cargo test -p agent-runtime --lib`: 249 passed (before the last additions; the handover and team tests were re-run after them and pass). The 19 failures are all environment limits:
  - `openpty` is blocked: 4 handover PTY tests (the watchdog end to end on a live process) and 2 pty tests;
  - Unix socket binding is blocked: otlp, pacing and `runtime::otlp_requests…`;
  - procs and worktree_watch (4 tests) fail the same way on untouched HEAD.
- orch-hook: 4 passed. The 7 tests that bind the fake core's socket can't run here, the new ack test included. orch-protocol: the 3 bus socket tests are blocked the same way.
- **Please run** `cargo test -p agent-runtime -p orch-store -p orch-hook` (and the e2e suite) outside the sandbox. To run them in the agent sandbox, allow local binding and PTYs (`sandbox.network.allowLocalBinding: true`).
- clippy on all touched crates with `--all-targets`: clean. `cargo fmt`: done.

## Known limits
- Step 3 sends a single Esc. If the paste text is still in the input box after an Enter retry, one Esc may leave it there, and the re-paste would append to it. That is rare and covered by the 3-attempt cap.
- The subagent check relies on `agent_id` on PostToolUse. It is not yet confirmed against the pinned CLI with a live test (there is no quota in tests here). The fallback is one line in `in_subagent`.
- While cleaning up a failed temporary `git worktree add`, I ran `git worktree prune`. It tried to prune the other worktrees, which the sandbox hides; every attempt was refused, and all 32 entries are still there.
