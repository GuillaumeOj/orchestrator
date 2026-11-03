# T-208 · Chaos fake CLI and e2e suite for F1–F9

Branch `orch/fix-team-getting-stucked/back-1`. Covers spec §3.8 and exit criteria 1 and 2.

## What was built

### The fake CLI misbehaves (`tests/fake_claude.sh`)
The spawned process gets a cleared environment, so the switches are **sentinel files in the agent's worktree** (the style FR-415 already uses), not env vars. A consumed one hits one turn.

| Switch | Effect |
|---|---|
| `.fake-swallow-enter` | the next Enter is lost; the text sits in the box until another Enter (consumed) |
| `.fake-modal` | a dialog eats every key until a lone Esc (kept, the test removes it) |
| `.fake-drop-hooks` | the next turn's hooks never reach the core; the transcript has it (consumed) |
| `.fake-drop-stop` | that turn's Stop never reaches the core (consumed) |
| `.fake-stop-failure` / `-sticky` | `StopFailure <content>` instead of Stop; for `rate_limit` the statusline then reads 20 % |
| `.fake-bg` | Stop carries a running background task (consumed) |
| `.fake-exit-mid-turn` / `.fake-crash-always` | the process dies after UserPromptSubmit |
| `.fake-delay-post <secs>` | PostToolUse waits (consumed) |
| `.fake-subagent` | the PostToolUse comes from a subagent (`agent_id`) (consumed) |
| `.fake-read-full` | the model reads any `full text: <path>` it is shown (kept) |
| `.fake-usage <pct>` | the statusline's 5 h usage (kept) |

Also: the fake writes an assistant `end_turn` line to the transcript (so the late-Stop recovery has something to read), ignores an empty Enter like the real CLI, and `Core` (tests/common) now points `projects_root` at the folder the fake writes its transcript to.

### Test knobs added to the core (no behaviour change by default)
- `Runtime::lift_limits` is `pub` (it takes `now`).
- Settings `delivery.invariant_secs` (default 10 min) and `delivery.slot_wait_secs` (default 2 min), next to `delivery.ack_secs`; shared reader `secs_setting`.
- `Runtime::inject_tick_fault(step)`: makes a team-tick step panic (the F9 test; the unit test still uses its own hook).

### `tests/e2e_delivery.rs` (16 tests; `Core` and `wait_for` moved to `tests/common`)
| Mode | Test | Bound shown |
|---|---|---|
| F1 | `f1_a_swallowed_enter_is_sent_again` | `redelivered{how:enter}`, marker on screen, next message arrives |
| F1 | `f1_a_modal_that_eats_keys_…` | Esc once, turn re-pasted, marker seen |
| F1 | `f1_a_lost_hook_is_applied_late_…` | `recovered{late_hook}`, no second paste |
| F1 | `f1_three_unacknowledged_pastes_…` | turn `failed`, one `delivery` decision (`no_ack`), next message arrives |
| F2 | `f2_a_server_error_gets_one_continue_…` | one Continue, second failure → `continue_failed` + decision |
| F2 | `f2_a_rate_limited_agent_is_idle_again_…` | `blocked_by_limit` → idle once the limit is over |
| F2 | `f2_background_tasks_do_not_hold_a_message` | idle with a background task, message delivered |
| F3 | `f3_a_crashed_agent_is_resumed_…` | `recovered{crashed, resume}`, message pasted again |
| F3 | `f3_an_agent_that_keeps_crashing_…` | 2 restarts, then `startup` decision `failed:crash`; the user's resume works |
| F4 | `f4_a_lost_stop_is_applied_…` | working watchdog (called with now + 16 min) applies the Stop |
| F5 | `f5_a_held_turn_does_not_block_…` | a Task held by the 5 h guard does not hold the user's answer |
| F6 | `f6_a_member_waiting_for_a_slot_…` | idle colleague suspended, waiter gets its message |
| F7 | `f7_a_long_message_is_read_only_…` | read in full when the file was read; else handed back once, then `read{truncated:true}` |
| F8 | `f8_an_injected_message_is_seen_once_…` | injected once at the tool call; a subagent's call gets nothing, the message arrives as a turn |
| F9 | `f9_a_panic_in_the_team_tick_…` | `tick_panic` event, delivery goes on |
| inv. | `the_delivery_invariant_names_the_cause_…` | one `delivery` decision (`no_ack`) per episode, closes when seen |


No real 20 s sleep anywhere: `delivery.ack_secs` is 1 s, the paste watchdog is driven from the wait loops, the 15 min watchdog is called with a later `now`, the invariant and slot waits are 1–2 s settings.

## Bugs found in T-204/206/207 code
None found, but nothing could run (see "Not run" below).

## Not run here
The sandbox of this session refuses `bind()` on a unix socket and `openpty()` (`Operation not permitted`, `out of pty devices`), so **no fake-claude e2e test could run here, old or new** (`idle_agents_are_suspended_and_resume_on_new_work` fails the same way on the unchanged code). What was checked:
- everything compiles (`cargo test --no-run`), `cargo fmt`;
- the fake's switches were driven over pipes with stub hooks (swallow, drop-hooks, drop-stop, StopFailure, background, exit, subagent, delay, modal's structure): each emits the hooks expected;
- not checked: the tty parts of the modal (`stty -icanon min 0 time 1` and the lone-Esc detection) and every timing in the tests.

The 5-runs loop (`for i in 1..5; cargo test -p agent-runtime --test e2e_delivery`) therefore has to run on the macOS CI or the user's machine. F6 and F5 depend most on Governor details (the 5 h guard at 90 %, the spawn held at `base_cap = 1`) and are the likeliest to need a tweak.
