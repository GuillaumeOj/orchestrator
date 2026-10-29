# Fix Flaky Tests · Feature spec

> Feature `fix-flaky-test-worktree-watch` · Integration branch `orch/fix-flaky-test-worktree-watch/integration` · Core only (no UI, no API change, no design pass)

## 1. Problem

Three tests fail now and then on `main` CI (macOS):

| Part | Test | Failure |
|---|---|---|
| A | `worktree_watch::tests::one_topic_per_change_two_per_burst_none_for_ignored` | `worktree_watch.rs:222` `ignored paths raise nothing`, left 1, right 0 |
| B | `an_unanswered_permission_parks_the_agent_and_allow_runs_the_retry` (`tests/e2e_fake_claude.rs`) | `e2e_fake_claude.rs:30` `timed out waiting for retried` |
| C | `otlp::tests::http_server_keepalive_chunked_and_rejections` | `otlp.rs:341` `post(rx.addr, "/v1/logs", …).starts_with("HTTP/1.1 200")` failed |

**Goal:** each passes every time on a loaded macOS runner and still checks what it was written to check. A and B are test bugs. **C is a real product bug** (the OTLP receiver), and the test caught it.

## 2. Base branch

`worktree_watch.rs` (Part A) came with p4.1, which is now on `origin/main` (7d8107d, PR #18). This integration branch was cut from the older local `main` (68ea76d), which lacks it. **Before any task starts, this integration branch must be brought up to `origin/main` (≥ 7d8107d).** Then all three parts are fixed against today's `main`, and task branches cut from or synced with integration have every file. The lead cannot merge `main` in (`merge` only takes this feature's branches), so this is done by the orchestrator or the user.

## 3. Part A: worktree watch (FSEvents)

### Cause

The test (`worktree_watch.rs:214-234`):

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

### Scope

Test-side fix in `worktree_watch.rs` (`mod tests`), plus at most a small, behaviour-neutral refactor of `start` if the test needs to see the raw paths:

- **Settle before asserting.** After `start`, wait until the watcher is quiet: clear `dirty`, and loop until no event has arrived for a quiet window (e.g. ≥ 500 ms), bounded (e.g. 5 s). Clearing `dirty` is required, not only sleeping.
- **Assert on the ignored path, not on "silence".** The "ignored" check must fail only if the `target/out.o` write itself counted. Preferred: the test records the paths that raised `dirty` (e.g. `start` takes an `on_change` sink, or the event filter is extracted into a function the test reuses) and asserts no recorded path is under `target/`; a stray `root`/rescan event is then tolerated and reported, not a failure. If a simpler approach is equally robust, the owner may choose it, but it must not just widen timeouts or loosen `0` to `<= 1`.
- **Diagnosable failures.** Every assertion message in this test prints the paths/events that raised a topic, so a future flake says what fired.
- Apply the same settle step to the next two phases (one write → exactly 1; burst → 1..=2) so a late event from the previous phase cannot turn the 1 into a 2. Keep the existing time budgets unless a change is justified.
- `a_write_in_a_watched_worktree_is_a_now_topic` only checks that a topic *is* seen, so it cannot fail on extra events; leave it as is unless the owner finds the same pattern.
- No change to `counts`, `Throttle`, `DEBOUNCE`, `IGNORED_DIRS` or the watcher's runtime behaviour.

Out of scope: the diff pane, the `now` pump, other tests.

### Acceptance

1. `cargo test -p agent-runtime worktree_watch` passes.
2. The FSEvents test passes **50 times in a row** under load (e.g. `for i in $(seq 50); do cargo test -p agent-runtime worktree_watch::tests::one_topic -- --exact … || break; done` while another `cargo test` runs in parallel). Note: the test needs the FSEvents service, which an agent sandbox may not have (the test comment says so). If the owner cannot run it, the report says so and gives the exact command; the lead or the user runs it.
3. Regression check: temporarily removing `"target"` from `IGNORED_DIRS` makes the "ignored" assertion fail (the test still detects a real bug).
4. `cargo clippy -p agent-runtime --all-targets` and `cargo fmt --check` are clean.

## 4. Part B: e2e permission retry

### Cause

`CtlRequest::Read` → `Runtime::read_output` → `PtyOutput::read` (`pty.rs:109`) **drains** the buffer: it returns "everything buffered since the last read", and that output is gone afterwards. The test's `screen()` helper calls it on each poll, and the failing wait is:

```rust
wait_for("retried", 30, || screen().contains("retry the call Bash pnpm add zod") && core.state_of("a") == "idle");
```

After Allow, the core pastes `ORCH: approved: retry the call Bash pnpm add zod` (`inbox.rs:659`), and fake_claude echoes it and runs the turn (PreToolUse, PermissionRequest, PostToolUse, Stop). The queued `other work` turn follows. If one 100 ms poll reads the retry text while the agent is still busy (the retry turn or `other work` is running), the `&&` is false, the text has already been drained, and no later poll can ever see it. The wait then times out after 30 s. Whether the poll lands in that window is pure timing, which explains the flake.

There's a smaller form of the same issue: a single `Read` can return half a line (the PTY gives chunks), so `contains` on one chunk can miss text that spans two reads.

### Scope

- The test's screen helper(s) **accumulate** output: each call appends what `Read` returned to a buffer kept for the test (per agent for the helper at ~line 515), and checks and assertion messages use the accumulated text. A helper that reads the newest text since a mark is fine too, if a check needs only new output.
- `wait_for("retried", …)` checks the retry text on the accumulated screen; the idle condition stays.
- Same treatment for the other `screen` helper in this file (the feature-run test, ~line 515: `screen(lead).contains(…)` waits and asserts), so a split chunk or an earlier drain can't make it flaky. Assertion messages that print `screen()` print the accumulated text, not a fresh (usually empty) drain.
- Test code only. No change to `PtyOutput::read`'s draining (the app's pane relies on it), the core or `fake_claude.sh`.

### Acceptance

1. `cargo test -p agent-runtime --test e2e_fake_claude` passes.
2. `an_unanswered_permission_parks_the_agent_and_allow_runs_the_retry` passes **30 times in a row** under parallel load (e.g. run the whole `e2e_fake_claude` file in a loop while another `cargo test` runs). If the sandbox blocks it (PTYs, socket), the report says so and gives the command.
3. A note in the report on why the fix is right: show (by reasoning or a temporary `sleep` in a local experiment) that a poll reading the text while the agent is busy no longer loses it.
4. `cargo clippy -p agent-runtime --all-targets` and `cargo fmt --check` are clean.

## 5. Part C: OTLP receiver (product bug)

### Cause

`OtlpReceiver::start` (`otlp.rs:125`) makes the **listener** non-blocking so the accept loop can poll `stop`. On macOS (BSD `accept(2)`), **an accepted socket inherits the listener's `O_NONBLOCK`** (Linux's `accept` does not, which is why it looks random). `serve_conn` (`otlp.rs:238`) only calls `set_read_timeout(IDLE_KEEPALIVE)`, which does nothing on a non-blocking socket. So every read in `read_request` returns `WouldBlock` as soon as the bytes it needs have not arrived yet. That `Err` becomes `400 Bad Request` and the connection is closed.

- **The test:** `post()` connects, then writes. The accept loop wakes every 100 ms. Usually the whole request is already buffered when the connection is accepted, so it passes. If the accept (or the first `read_line`) runs between `connect` and the end of the `write`, the server answers 400, and line 341 fails. The keep-alive half has the same race: the second request may not have arrived when the server goes back to read.
- **In the app:** Claude Code's OTLP exporter keeps connections alive. After each request, the next `read_line` on the idle connection gets `WouldBlock` at once, so the receiver sends an unsolicited `400` and closes. Keep-alive and the 10 s idle timeout don't work, and a request arriving in pieces (a large log batch over several TCP segments) can be rejected and its usage samples lost.

I could not reproduce this here: the lead's sandbox can't bind local ports. The cause comes from the code plus BSD semantics, and it matches the symptom (a non-200 on the first `post`). The owner confirms it (acceptance 2).

### Scope

- `serve_conn` puts the accepted stream back in blocking mode (`set_nonblocking(false)`) before setting the read timeout. If that call fails, the connection is dropped. This is the product fix, a couple of lines in `otlp.rs`. No other behaviour change.
- **Regression tests** that fail deterministically on macOS without the fix:
  - a client that connects, waits (e.g. 200-300 ms), then sends a request, and gets `200`;
  - a request whose body is written in two parts with a pause between them, and gets `200`;
  - on one keep-alive connection: a request, a pause (e.g. 300 ms, well under `IDLE_KEEPALIVE`), a second request, and two `200`s with no stray `400`.
- The existing test keeps its checks; failure messages print the actual response (today it is a bare `assert!` that hides it).
- `otlp.rs:127` is the only `set_nonblocking(true)` listener in `crates/` on `origin/main` (checked), so no other site needs the fix.

### Acceptance

1. `cargo test -p agent-runtime otlp` passes.
2. The new regression tests **fail** with the fix reverted and pass with it (on macOS). The report shows both runs. If the agent sandbox blocks local port binding, the report says so and gives the exact command; the lead or the user runs it.
3. The OTLP tests pass 50 times in a row under parallel load.
4. `cargo clippy -p agent-runtime --all-targets` and `cargo fmt --check` are clean.

## 6. Tasks

| Task | Owner | Files | Notes |
|---|---|---|---|
| T-1 Worktree-watch test settles and checks only the ignored path (A) | back | `crates/agent-runtime/src/worktree_watch.rs` (tests, optional sink refactor of `start`) | |
| T-2 E2E screen helpers stop losing drained output (B) | back | `crates/agent-runtime/tests/e2e_fake_claude.rs` | |
| T-3 OTLP receiver serves accepted connections in blocking mode (C) | back | `crates/agent-runtime/src/otlp.rs`  | product fix + regression tests |

All three start once integration is on `origin/main` (§2). They touch different files, so they don't conflict. `back` takes them in the order T-3 (product bug), T-2, T-1. `front` reviews each one. Several checks need real FSEvents, PTYs or local ports, which an agent sandbox may block. When that happens the report gives the exact loop command and the lead or the user runs it before the PR.
