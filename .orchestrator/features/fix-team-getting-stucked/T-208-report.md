# T-208 · Chaos fake CLI and e2e suite for F1–F9

Branch `orch/fix-team-getting-stucked/back-1`. Covers spec §3.8 and exit criteria 1 and 2.

## Approach (changed after the first report)
The first version ran everything through `fake_claude.sh` on a real PTY and a unix socket, which this sandbox refuses (`openpty`, `bind`). The suite is now **in process**, and runs in the sandbox with `ORCH_TEST_TMP=$TMPDIR TMPDIR=$TMPDIR cargo test -p agent-runtime --lib chaos::`.

### The terminal seam
- `PtyProcess::fake(on_input, on_exit)` (pty.rs): a terminal with no process. Writes go to `on_input`; output is pushed with `output.push`. Its pid is one no process has, so no signal reaches anything. `fake_exit` / `fake_handle` end it; `Runtime::stop` calls `fake_exit` so suspend/kill work on it.
- `RuntimeConfig.terminal: Option<TerminalFactory>` replaces the PTY + `claude` of a spawn (`Runtime::spawn`). `None` in production.
- `insert_test_live` now uses an echoing fake terminal, so the 4 `openpty`-based handover watchdog tests run in the sandbox (they pass; no change to their assertions).

### The fake CLI (`src/chaos.rs`, `#[cfg(test)]`)
A thread behind the fake terminal. It takes a bracketed paste, Enter and Esc, echoes like a tty, and answers the way Claude Code does: through `Runtime::serve_hook` (plus the `on_hook_ack` orch-hook sends), `on_status` and a transcript file. Switches (set per test on `Cli`): swallow the next Enter, a modal (once or `modal_always`) that eats keys until Esc, dropped hooks, dropped Stop, StopFailure (`rate_limit`, `server_error`, sticky), Stop with a background task, exit mid-turn / always, a delayed PostToolUse, a subagent's PostToolUse, the model reading `full text:` files, the statusline's 5 h usage.
`Sim` builds a runtime whose spawns start this CLI, a git worktree for agent `a1`, a squad run past its gate, and the loop of the server (`pump`, `team_tick`, `paste_watchdog`, `housekeeping`). Once every fake CLI is quiet the paste watchdog gets a clock 30 s ahead, so its 20 s bound costs nothing.

### Scenarios (16, all pass; 6 runs in a row)
F1 ×4 (swallowed Enter, modal + Esc, lost hook applied late, three unacknowledged pastes → `delivery` decision), F2 ×3 (server error → one Continue, second failure → decision; rate limit lifted; background tasks), F3 ×2 (crash resumed; two restarts then a `startup` decision, user resume works), F4 (lost Stop from the transcript), F5 (held Task does not block the user's answer), F6 (slot freed for a waiting member), F7 (long message read only when seen, else truncated once), F8 (injection seen once, never into a subagent), F9 (`tick_panic`, delivery goes on), and the delivery invariant (one decision per episode, closes when seen).

### CI smoke tests (`tests/e2e_delivery.rs`, `fake_claude.sh`)
`fake_claude.sh` keeps its switches (sentinel files in the worktree). Three smoke tests remain for CI: a swallowed Enter (F1), a server error (F2), a crashed agent resumed (F3). They need a PTY and a socket, so they do not run in this sandbox; they were not run here.

### Other knobs
- Settings `delivery.invariant_secs` (default 10 min) and `delivery.slot_wait_secs` (default 2 min), next to `delivery.ack_secs`; `Runtime::lift_limits` and `inject_tick_fault` are `pub`.

## Bugs found in T-204/206/207 code
None. Two things worth knowing from building the simulation: a limit placed by a StopFailure is cleared only by a **newer** reading of the same window with the same reset (the fake sends 100 % then 20 %, strictly later), and a crash mid-turn leaves the interrupted turn `Delivered` (the next message restarts the agent; that orphan turn is not requeued).

## First version (kept)

### What was built

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

