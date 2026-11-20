# Fix flaky test: a_failed_switch_stop_lets_the_task_run_on_the_current_model · Feature spec

> Feature `fix-flaky-test-a_failed_switch_stop_lets` · Integration branch `orch/fix-flaky-test-a_failed_switch_stop_lets/integration` · Core only (no UI, no API change, no design pass)

## 1. Problem

`pacing::tests::a_failed_switch_stop_lets_the_task_run_on_the_current_model` (`crates/agent-runtime/src/pacing.rs`) fails now and then on `main`:

```
panicked at crates/agent-runtime/src/pacing.rs:414:9:
asked once
```

Line 414 is the last assertion, `assert!(rt.house().switch_failed.is_empty(), "asked once")`. The assertion before it (`!task_boundary(..)`, "it runs on the current process") passed.

## 2. Cause

The test does this:

1. The first `task_boundary(a1)` call: the live process runs sonnet and the task wants opus. The agent is idle, so it calls `suspend_in_background(a1, "model_switch")`. This **spawns a real background thread** that runs `suspend` → `stop`.
2. The test inserts `a1` into `switch_failed` to simulate a failed stop.
3. The second `task_boundary(a1)` call is expected to consume the flag (`switch_failed.remove`) and return false.

Since 137c0b7f ("the test live process exits through on_exit like a real one"), the fake PTY from `insert_test_live` calls `Runtime::on_exit`. So the background suspend from step 1 now **succeeds quickly and removes the `live` entry**. When that thread finishes before step 3, `task_boundary` reaches

```rust
let Some(l) = live else { return false };
// ...
let switch_failed = self.house().switch_failed.remove(&rec.id);
```

and returns false at the early return, **before** it removes the flag. The second assertion passes anyway (false). The flag is still set, so "asked once" fails.

The test itself is racy: it simulates a failed stop while a real stop runs at the same time and succeeds.

### Product impact (small)

The same early return can leave a stale flag in production. Say a switch stop fails (the flag is set) and the process then exits by itself before the next boundary. The flag stays set. The next boundary with a new live process skips one switch it should have made. Rare, but it is the same ordering bug.

## 3. Fix

1. **`task_boundary` (pacing.rs):** consume `switch_failed` (the `remove`) **before** `let Some(l) = live else { return false }`. The flag belongs to the boundary that observes it. With no live process it is moot (the next spawn uses the wanted model), so it must not outlive the call. No other behaviour change.
2. **The test (deterministic):** stop the first `task_boundary` from spawning a real suspend. Set `tracker.stop_intent = StopIntent::Suspend` before the first call: `suspend_in_background` then returns false without a thread, and `task_boundary` still returns true ("the task waits"). Then do what the failure path of the background thread does: reset `stop_intent` to `None` and insert `a1` into `switch_failed`. The second call must return false, the flag must be empty, and `live` must be untouched (still there, and `l.task` set to the seat's task, so `keeps_model` holds).
3. **A regression test for the ordering:** a stale `switch_failed` with no live process is cleared by `task_boundary` (it returns false and the set is empty). It must fail on the old code.

## 4. Tests / done when

- The rewritten test and the new one pass. The new one fails on the old code (say so in the report).
- `cargo test -p agent-runtime --lib pacing` green, run in a loop a few dozen times (it may not reproduce locally; the deterministic test is what counts).
- `cargo test --workspace` green (socket/pty tests may fail in the sandbox; CI judges those). `cargo clippy --workspace --all-targets` clean. `cargo fmt --check` clean.

## 5. Team

One task for `back` (pacing.rs and its tests). No work for `front`.
