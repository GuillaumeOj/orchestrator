# Fix Flaky Test Worktree Watch · Feature spec

> Feature `fix-flaky-test-worktree-watch` · Integration branch `orch/fix-flaky-test-worktree-watch/integration` · Core test only (no UI, no API change, no design pass)

## 1. Problem

CI fails now and then on:

```
worktree_watch::tests::one_topic_per_change_two_per_burst_none_for_ignored
panicked at crates/agent-runtime/src/worktree_watch.rs:222:9:
assertion `left == right` failed: ignored paths raise nothing
  left: 1
 right: 0
```

**Goal:** the test passes every time on a loaded macOS CI runner and still proves what it was written for: ignored paths raise no topic, one write raises one topic, a burst raises at most two.

## 2. Where the code is (base-branch caveat)

`crates/agent-runtime/src/worktree_watch.rs` is **not on `main`** (neither local `main` 68ea76d, the base of this integration branch, nor `origin/main` 9daeb83). It was added by T-33 on feature **p-4-1-one-now** and exists only on `orch/p-4-1-one-now/{integration,back-1,front-1}`. The failing CI run is that feature's branch / PR. The fix has to land on a branch that has the file: see §6.

## 3. Cause

The test (lines 214-234 on `orch/p-4-1-one-now/integration`):

```rust
let (_d, root) = tmp();                                 // creates the temp dir
std::fs::create_dir_all(root.join("target")).unwrap();  // creates target/
let mut t = Throttle::default();
let _w = start(&root, t.dirty.clone()).unwrap();        // FSEvents stream, SinceNow
std::thread::sleep(Duration::from_millis(300));
std::fs::write(root.join("target/out.o"), "x").unwrap();
assert_eq!(pump_for(&mut t, 1600 ms), 0, "ignored paths raise nothing");
```

The production filter is right (`what_counts` covers `target/…` → ignored). The extra topic comes from an FSEvents event that is **not** the `target/out.o` write. Any event whose path is `root` or a non-ignored child of it counts, and so does any `notify` error (`res.map_or(true, …)`). On a loaded runner:

1. **Late setup events.** The temp dir and `target/` are created a few ms before the stream starts. `kFSEventStreamEventIdSinceNow` only means "events fseventsd has not flushed yet": events that happened just before the stream was created can still be delivered, and the one for the temp dir itself has path `root`, which counts (`c("/w/back-1")` is `true` by design). The 300 ms sleep does not drain them: it only waits; it never clears `dirty`, so an event that landed during the sleep is still there when the pump starts.
2. **Coalesced / rescan events.** Under load FSEvents may drop file-level events and send a `MustScanSubDirs` event (notify: `Flag::Rescan`) for a directory, often `root`, or `notify` may report an error. Both raise `dirty` on purpose (a refetch is the safe answer).

Both are correct product behaviour (one extra diff refetch is harmless); the test is what assumes a perfectly quiet stream.

## 4. Scope

Test-side fix in `worktree_watch.rs` (`mod tests`), plus at most a small, behaviour-neutral refactor of `start` if the test needs to see the raw paths:

- **Settle before asserting.** After `start`, wait until the watcher is quiet: clear `dirty`, and loop until no event has arrived for a quiet window (e.g. ≥ 500 ms), bounded (e.g. 5 s). Clearing `dirty` is required, not only sleeping.
- **Assert on the ignored path, not on "silence".** The "ignored" check must fail only if the `target/out.o` write itself counted. Preferred: the test records the paths that raised `dirty` (e.g. `start` takes an `on_change` sink, or the event filter is extracted into a function the test reuses) and asserts no recorded path is under `target/`; a stray `root`/rescan event is then tolerated and reported, not a failure. If a simpler approach is equally robust, the owner may choose it, but it must not just widen timeouts or loosen `0` to `<= 1`.
- **Diagnosable failures.** Every assertion message in this test prints the paths/events that raised a topic, so a future flake says what fired.
- Apply the same settle step to the next two phases (one write → exactly 1; burst → 1..=2) so a late event from the previous phase cannot turn the 1 into a 2. Keep the existing time budgets unless a change is justified.
- `a_write_in_a_watched_worktree_is_a_now_topic` only checks that a topic *is* seen, so it cannot fail on extra events; leave it as is unless the owner finds the same pattern.
- No change to `counts`, `Throttle`, `DEBOUNCE`, `IGNORED_DIRS` or the watcher's runtime behaviour.

Out of scope: the diff pane, the `now` pump, other tests.

## 5. Acceptance

1. `cargo test -p agent-runtime worktree_watch` passes.
2. The FSEvents test passes **50 times in a row** under load (e.g. `for i in $(seq 50); do cargo test -p agent-runtime worktree_watch::tests::one_topic -- --exact … || break; done` while another `cargo test` runs in parallel). Note: the test needs the FSEvents service, which an agent sandbox may not have (the test comment says so). If the owner cannot run it, the report says so and gives the exact command; the lead or the user runs it.
3. Regression check: temporarily removing `"target"` from `IGNORED_DIRS` makes the "ignored" assertion fail (the test still detects a real bug).
4. `cargo clippy -p agent-runtime --all-targets` and `cargo fmt --check` are clean.

## 6. Base branch (decision for the user)

Because the file only exists on `orch/p-4-1-one-now/*`, the change cannot be made on this integration branch as it stands. Options:

- **A (recommended):** merge `orch/p-4-1-one-now/integration` into this integration branch first, do the fix here, and open this PR **after** p4.1's PR lands on `main` (then its diff is only the test fix). Or stack it on p4.1's PR.
- **B:** hand the fix to the p-4-1-one-now team so it ships inside the p4.1 PR whose CI is failing; this feature is closed without a PR.
- **C:** wait until p4.1 is merged into `main`, sync this integration branch, then do the fix.

## 7. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Make the FSEvents worktree-watch test deterministic | back | `crates/agent-runtime/src/worktree_watch.rs` tests (+ optional sink refactor of `start`) |

No frontend work: `front` reviews the change.
