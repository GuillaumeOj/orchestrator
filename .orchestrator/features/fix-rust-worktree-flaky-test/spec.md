# Fix Rust Worktree Flaky Test · Feature spec

> Feature `fix-rust-worktree-flaky-test` · Integration branch `orch/fix-rust-worktree-flaky-test/integration` · Core only, test code only (no UI, no API change, no product change)

## 1. Problem

`worktree_watch::tests::one_topic_per_change_two_per_burst_none_for_ignored` still fails now and then on `main`:

```
panicked at crates/agent-runtime/src/worktree_watch.rs:267:9:
assertion `left == right` failed: fired: ["\"/private/tmp/orchwtMnfQp7/a.txt\"", "\"/private/tmp/orchwtMnfQp7/a.txt\"", "\"/private/tmp/orchwtMnfQp7/a.txt\""]
  left: 2
 right: 1
```

The earlier fix (`fix-flaky-test-worktree-watch`, 2d68306) handled late setup events and stray events in the *ignored* phase. This is a different failure, in the *one write* phase (line 265-267).

## 2. Cause

What fired: three events, **all for `a.txt`**. So `settle` worked (nothing from setup or the earlier phase), and the filter worked. One `std::fs::write` produced events that arrived in at least two deliveries, and the later one came more than `DEBOUNCE` (1 s) after the first topic:

- notify 8.2 runs FSEvents with `latency: 0.0` and `kFSEventStreamCreateFlagFileEvents | NoDefer`. The first callback fires as soon as fseventsd has something. FSEvents flags are cumulative per path, and fseventsd can deliver the same path again later (Created, then Created|Modified, then an inode-meta or xattr change). notify splits each callback into one event per flag, which is why one write shows as three entries.
- On a loaded runner (the full suite runs 249 tests in about 50 s), the second delivery can come more than 1 s after the first. Then `Throttle` does what it was built to do: a topic at once, then a trailing one a second later.

**The product is right.** Its contract (module doc) is "at once and then at most once a second (a burst ends with a trailing one)". Late events for a path that changed lead to one extra diff refetch, which is harmless and intended. The test is wrong to expect a write's events to arrive within one second, because the OS doesn't promise that. The burst phase (`1..=2` within 2.5 s) depends on the same assumption, and fails the same way if events arrive more than 2 s apart.

## 3. Fix (test only)

Stop counting topics against a fixed number. Check the throttle contract against the events that **actually arrived**. Record *when* each event arrived and *when* each topic was raised, and assert:

1. **Seen:** the phase raises at least one topic.
2. **At once:** the first topic is raised soon after the first event (within one pump period plus slack, say ≤ 250 ms with the 50 ms pump).
3. **Never without a change:** each topic after the first comes after at least one event that arrived after the previous topic.
4. **At most once a second:** two topics are always ≥ `DEBOUNCE` apart.
5. **Only the expected paths:** every recorded event in the phase is the phase's own file(s) (`a.txt`; `b*.txt` for the burst). An event elsewhere (a rescan of `root`, a notify error) is printed in the message but doesn't fail the test, as the ignored phase already does.

Each check holds whatever the delivery timing is, and each still catches a real regression: a throttle that never fires, one that fires more than once a second, one that fires with no change, or one that is late to fire.

Implementation guidance (the owner may choose another shape if it checks the same things):

- The `watch` sink in the test records `(Instant, String)`. Push the entry **before** setting `dirty`, so every event that `due` sees is already recorded.
- `pump_for` (or a variant) returns the `Instant`s of the topics it raised, not only a count.
- One helper, e.g. `assert_throttled(events, topics, expect: impl Fn(&str) -> bool)`, runs checks 1-5 for both phases. Every failure message prints the timeline (events and topics, as ms offsets from the write) so a future failure says exactly what happened.
- Keep the ignored phase as it is (it already asserts on paths). Keep `settle` between phases.
- The burst phase keeps its tight shape (50 writes over ~200 ms). The `1..=2` count goes away: checks 3 and 4 replace it. The report should still print the topic count.
- The pump windows (1.5 s, 2.5 s) may stay as they are: they only bound how long the test watches.
- Rename the test if its name no longer fits (e.g. `topics_follow_the_throttle_contract_none_for_ignored`), and update its doc comment.

Out of scope: `counts`, `Throttle`, `DEBOUNCE`, `IGNORED_DIRS`, `watch`/`start` behaviour, the other tests in the file (`a_write_in_a_watched_worktree_is_a_now_topic` only checks that a topic is seen, so it can't fail on late events).

## 4. Acceptance

1. `cargo test -p agent-runtime worktree_watch` passes.
2. The FSEvents test passes **50 times in a row under load**: loop it with `-- --exact` while a full `cargo test --workspace` runs in parallel. The test needs the FSEvents service, which an agent sandbox may not have. If the owner can't run it, the report says so and gives the exact command, and the lead or the user runs it before the PR.
3. **Regression checks**, run locally and reverted, results in the report:
   - `Throttle::due` ignoring `last` (no debounce): check 4 fails.
   - `Throttle::due` not clearing `dirty` (`load` instead of `swap`): check 3 fails.
   - `"target"` removed from `IGNORED_DIRS`: the ignored assertion fails (as today).
4. `cargo clippy -p agent-runtime --all-targets` and `cargo fmt --check` are clean.

## 5. Tasks

| Task | Owner | Files | Review |
|---|---|---|---|
| T-1 The FSEvents watcher test checks the throttle contract against the real event timeline | back | `crates/agent-runtime/src/worktree_watch.rs` (`mod tests` only) | front |

There's one task, so there's no ordering and no conflicts. No `ux-brief.md` or `api-contract.md`: no UI and no API.
