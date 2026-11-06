# Fix Flaky Test Pacing

## Problem

`pacing::tests::a_task_model_switches_at_the_task_boundary_and_back_at_the_next_task`
(`crates/agent-runtime/src/pacing.rs`) fails sometimes on main with
`assertion failed: stopped()` (line 444). It fails more often when the
whole suite runs under load (the failing run took 75 s).

## Root cause

The test's live process comes from `insert_test_live`
(`crates/agent-runtime/src/runtime.rs`, test module). That helper builds an
`echoing_fake()` terminal whose exit callback does nothing (`Box::new(|_| {})`).

`task_boundary` asks for a suspend with `suspend_in_background`. That
spawns a thread that runs `stop(id, Suspend)`:

1. `kill_tree` on the fake pid (2 000 000 000+, not a real process): it
   returns at once.
2. `l.pty.fake_exit(Some(143))`: it calls the no-op callback. Nothing calls
   `Runtime::on_exit`, so the `live` entry is never removed.
3. The bounded wait (`for _ in 0..100 { … sleep(50ms) }`) waits for the
   live entry to go away. It never does, so the wait always runs its full
   ~5 s (more under load: 100 sleeps plus the lock and store reads).
4. Only then does it write the `stopped` event.

The test polls for `stopped` for 10 s, counted from after its earlier
asserts. The nominal margin is 5 s, and that margin is used up when the
machine is loaded (parallel test threads, thread-spawn delay, oversleeps,
the event writer). The test also costs ≥5 s on every run.

So the bug is in the test fixture: the fake process does not "exit" like a
real one does. The production path is correct. A real process's exit calls
`on_exit`, which removes the live entry, and the wait ends at once.

## Fix

1. **Make the test fake exit like a real process.** In the test module of
   `runtime.rs`, `insert_test_live` builds a fake whose exit callback calls
   `Runtime::on_exit(id, pid, status)` through the runtime's `Weak`
   (`rt.me`). A stop of a test live agent then removes the live entry and
   returns within milliseconds, and the `stopped` event follows right away.
   - Check the other users of `insert_test_live` (`pacing.rs` tests and
     `handover/tests.rs`) still pass and still test what they say. A test
     that relied on the live entry staying after an exit has to be made
     explicit.
   - Keep `echoing_fake`'s echo behaviour (output gets what is typed).
2. **Make the test robust, not just faster.** In
   `a_task_model_switches_at_the_task_boundary_and_back_at_the_next_task`:
   - Wait for `stopped` with a generous deadline (e.g. 30 s), so a slow CI
     machine is not a failure. With fix 1 the normal wait is milliseconds.
   - Also assert the live entry is gone once `stopped` is seen (the stop
     really ended the process).
3. **Tidy up.** The three pacing tests that call `tempdir_in("/tmp")` use
   `orch_protocol::test_tmp()` like the rest of the suite (it honours
   `ORCH_TEST_TMP` in a sandbox).

No production code changes. If the work shows a production change is
needed (e.g. `stop` mishandles an exit that already happened), report it to
the lead first.

## Acceptance

- The test passes 50 runs in a row under load. Run
  `cargo test -p agent-runtime pacing::tests::a_task_model_switches -- --test-threads=1`
  in a loop while a full `cargo test -p agent-runtime` runs in parallel (or
  an equivalent stress), and record the result in the task report.
- The test now takes well under 1 s (it was ≥5 s).
- `cargo test -p agent-runtime` passes in full, and `cargo clippy --all-targets`
  is clean for the crate.

## Out of scope

- Other flaky tests. Report any you see, but don't fix them here.
- Changing the 5 s bounds in `stop` / `kill_tree`.

## Team

Backend only (Rust, `crates/agent-runtime`). Frontend has no work in this
feature. No design and no API contract.
