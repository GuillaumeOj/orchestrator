# Fix flaky test: `f3_a_crashed_agent_is_resumed_and_gets_its_message`

> This folder held the spec of an earlier feature with the same slug (the flaky
> `retry_on_an_invariant_decision…` scenario, commit ec03f9cf); it is in git history.

## Symptom (CI, main)

```
thread 'chaos::scenarios::f3_a_crashed_agent_is_resumed_and_gets_its_message'
panicked at crates/agent-runtime/src/chaos.rs:721:9:
assertion failed: cli.saw(&format!("[t:{t}]"))
```

`sim.until("the turn done", …)` passed (the core has the turn `Done`), and the next line
finds no screen line with the turn's `[t:<id>]` marker.

## Root cause (lead's analysis, to be confirmed by `back`)

The test harness races itself, not the product.

In the fake CLI (`Machine::turn`, `chaos.rs` ~line 233–307) the order of a normal turn is:

1. transcript `user` line, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, statusline,
   transcript `assistant` line;
2. the `Stop` (or `StopFailure`) hook, served synchronously by `Runtime::serve_hook`:
   `end_turn` marks every delivered turn **`Done`** in the store;
3. only then `self.say(format!("DONE[{n}] {prompt}"))`, the only screen line that carries
   the prompt, so the only one with `[t:<id>]`.

The test thread polls `sim.status(&t) == Done` every step (~20 ms). When it reads the
store between 2 and 3 (the CLI thread preempted right after the hook returned, which a
loaded CI runner with 620 tests does), `until` returns and `cli.saw("[t:…]")` is false.

The same race exists in every assertion of the pattern "turn `Done` → the model saw its
marker": `Sim::next_message_arrives` (line ~583, called by most F-scenarios), F1 at lines
~615 and ~628, F3 at line 721. F3 is simply the one that lost the race this time.

Checked and ruled out: `continue_after_crash` closing the new turn as `Done` before the
new process ran it. It runs synchronously in `on_exit` on the old CLI thread, before the
resumed process can even pass `SessionStart` (150 ms start + `AFTER_SESSION_START`
1.5 s), so the user's turn is never `Delivered` when it runs.

## Fix

Test-harness only, in `crates/agent-runtime/src/chaos.rs`:

- In `Machine::turn`, show the turn's `DONE[{n}] {prompt}` line **before** sending the
  `Stop` / `StopFailure` hook (and in the `drop_stop` case as today). This is also what
  the real Claude Code does: the reply is on screen before the Stop hook fires. A turn
  the core sees as `Done` has then always been seen by the model.
- The crash path (`exit` after `UserPromptSubmit`) is unchanged: it shows no `DONE` line.
- Update the `Cli::screen` doc comment to say the `DONE` line comes before the Stop.
- Check the screen-line assertions that read after an `until(saw …)` (F4 line ~782,
  F5 ~804, F8 ~878–894, the send-order test ~1009–1011, F7, F9, the team scenarios) still
  mean the same; none of them should need a Stop to have been applied when the line shows,
  but if one does, wrap its follow-up check in an `until` rather than reorder again.
- Give the four bare `assert!(cli.saw(&format!("[t:{t}]")))` the same failure message as
  `next_message_arrives` (`"the model saw {t}: {:?}", cli.lines()`), so a future failure
  prints the screen.

No production code change. If the reproduction shows a different cause, report to `lead`
before touching `runtime.rs` / `delivery.rs`.

## Verification

- Reproduce first, deterministically: put a `std::thread::sleep(Duration::from_millis(200))`
  between the `Stop` hook and the `DONE` line (temporary, not committed) and run
  `cargo test -p agent-runtime --lib chaos::scenarios::f3_a_crashed_agent_is_resumed` —
  it must fail with the CI assertion. With the fix (and the same sleep moved after the
  hook, i.e. after the `DONE` line), it must pass.
- Then, without the sleep: `cargo test -p agent-runtime --lib chaos::` passes, run in a
  loop (≥ 20 runs) to show no other scenario turned flaky, and the full
  `cargo test -p agent-runtime --lib` passes (`ORCH_TEST_TMP=$TMPDIR/t`; socket/pty tests
  may fail in the sandbox — CI judges them).
- `cargo clippy` and `cargo fmt --check` clean.

## Team

One task, `back` (Rust test harness). No frontend or design work: `front` has nothing to
do on this feature, so no `api-contract.md` / `ux-brief.md`.
