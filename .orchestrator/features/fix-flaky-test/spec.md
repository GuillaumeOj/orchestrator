# Fix flaky test: `retry_on_an_invariant_decision_resends_and_a_second_failure_is_raised_again`

## Symptom (CI, main)

```
thread 'chaos::scenarios::retry_on_an_invariant_decision_resends_and_a_second_failure_is_raised_again'
panicked at crates/agent-runtime/src/chaos.rs:970:9:
assertion failed: sim.items("delivery").is_empty()
```

The last assertion of the scenario: after the second Retry worked and the lead saw the
message, a `delivery` decision is still open.

## Root cause (lead's analysis, to be confirmed by `back`)

The scenario runs with `short_bounds()`: `delivery.invariant_secs` = 1 s for the whole test.

1. Line 966–968: the test lifts the modal, sets `ack_secs` = 1 and retries the second decision.
   `retry_delivery` closes it, requeues the paste, sets `ready_at = now + AFTER_STOP (500 ms)`
   and restarts the message's invariant clock (`retried_at = now`).
2. Until the lead's `UserPromptSubmit` arrives the lead is not `Working` and the message is
   unread. If that takes more than 1 s (500 ms `AFTER_STOP` + pump/tick cadence + the fake
   CLI thread, under CI load), `delivery_invariant` (team tick) raises a **third**
   `stuck_lead` decision: the retry released the episode key, so this is a new episode.
3. The message is then seen; the `until("the message seen")` condition turns true on the
   CLI thread right after a tick. `close_delivered` (first step of the next team tick)
   would close the third decision by itself, but no tick runs before line 970 asserts.

So the product behaves as specified (a decision raised during a slow redelivery closes
itself once the message is seen). The flake is the test: a 1 s invariant window racing
a real-clock redelivery, and a strict assertion with no tick between "seen" and "no open
decision". Production uses 10 min, where this race does not exist.

## Fix

Test-only, in `crates/agent-runtime/src/chaos.rs` (scenario at line ~946):

- Before the second (successful) Retry, set `INVARIANT_KEY` to a large value (e.g. 3600 s),
  as the scenario already does for `ACK_KEY` in its first half. No new invariant episode
  can then open while the retry goes through, and the final strict assertion keeps its
  meaning: *when the retry works, the decision stays closed* (nothing new is raised).
- Keep the final assertion strict (do not fold it into the `until`, which would hide a real
  "decision raised again" regression).
- Check the sibling scenarios using the same pattern
  (`the_delivery_invariant_names_the_cause_of_a_message_left_unread`, the F-series using
  `short_bounds`) for the same race; fix only those that can race the same way.

No production code change unless the reproduction shows a different cause (then report to
lead before changing `stuck.rs` / `handover.rs`).

## Verification

- Reproduce first: run the scenario repeatedly under load (e.g. many parallel copies:
  `cargo test -p agent-runtime retry_on_an_invariant_decision -- --test-threads=1` in a loop
  while the rest of the chaos suite runs, or `for i in $(seq 50)` with CPU pressure) and,
  on failure, print the open item's `payload` (episode, blocker, key) to confirm it is a
  third `stuck_lead` raised after the retry.
- After the fix: the same loop passes (≥ 50 runs under load), and the whole
  `chaos::scenarios` module and `cargo test -p agent-runtime` pass
  (`ORCH_TEST_TMP=$TMPDIR/t`; socket/pty tests may fail in the sandbox — CI judges them).
- `cargo clippy` / `cargo fmt` clean.

## Team

- `back`: the whole fix (test code in `agent-runtime`). One task.
- `front`: no work.

## Out of scope

Changing the invariant or retry semantics in production code.
