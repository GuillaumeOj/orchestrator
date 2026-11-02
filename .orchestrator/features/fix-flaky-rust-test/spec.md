# Fix Flaky Rust Test · Feature spec

> Feature `fix-flaky-rust-test` · Integration branch `orch/fix-flaky-rust-test/integration` · Core only (no UI, no API change, no design pass)

## 1. Problem

`idle_agents_are_suspended_and_resume_on_new_work` (`crates/agent-runtime/tests/e2e_fake_claude.rs:298`) fails now and then on `main` (macOS):

```
panicked at crates/agent-runtime/tests/common/mod.rs:48:46:
called `Result::unwrap()` on an `Err` value: io: Invalid argument (os error 22)
```

Line 48 is `request(sock, CtlRequest::State).unwrap()` in the `agent()` helper, which the test polls every 100 ms. The test logic is fine. **The client side of the control socket has a race, and the same race is in production code.**

## 2. Cause

`orch_protocol::bus::round_trip` (`crates/orch-protocol/src/bus.rs:156`):

```rust
let s = connect(path, connect_timeout)?;
write_line(&mut &s, msg)?;               // 1. send the request
s.set_read_timeout(reply_timeout)?;      // 2. setsockopt(SO_RCVTIMEO)   ← EINVAL
let reply = read_line(...)?;             // 3. read the reply
```

The server (`server::handle`) reads the line, writes the reply and **drops the stream** right away. A `State` request is quick, so on a loaded machine the server can finish between steps 1 and 2. On macOS, `setsockopt` on a socket whose peer has already closed fails with `EINVAL` (setsockopt(2): "the socket has been shut down"). `BusError::from_io` maps unknown kinds to `BusError::Io`, which prints as `io: Invalid argument (os error 22)`. The reply is waiting in the receive buffer, but it is never read.

Reproduced here with a socketpair (bind is blocked in the lead's sandbox):

| Order | Result |
|---|---|
| peer replies and closes, then `setsockopt(SO_RCVTIMEO)` | `[Errno 22] Invalid argument` |
| `setsockopt(SO_RCVTIMEO)`, then peer replies and closes | reply read ✓ |

Linux allows this call, so the race only shows on macOS.

### Same race elsewhere

| Call site | Who uses it | Impact |
|---|---|---|
| `bus::round_trip` (bus.rs:164) | `server::request` (every `orchd` / app ctl call, every test), `orch-mcp` (`lib.rs:304`) | ctl call or MCP tool call fails at random with `io: Invalid argument` |
| `orch-hook` `send` (`orch-hook/src/lib.rs:120`) | every Claude hook | a hook with a quick reply (`Ack`, a quick allow) can fail. The hook then takes its error path (spool / fallback) even though the core answered |

`runtime.rs:2645` (`conn_closed`) ignores the result (`let _ =`) and is meant to detect a closed peer, so it stays as it is.

## 3. Fix

**Set the read timeout before writing the request.** The server only replies after it has read the request, so before the write the peer cannot have closed (in normal operation).

1. `bus::round_trip`: move `s.set_read_timeout(reply_timeout)?` to just after `connect`, before `write_line`.
2. `orch-hook` `send`: work out `left` and set the read timeout before `write_line`. The "deadline already gone" check (`left.is_zero()` → `Timeout`) stays before the write, so nothing is sent when no time is left. The time spent writing one line is not subtracted any more. That is fine: a local socket write takes microseconds, and the hook's outer deadline still applies.
3. No other behaviour change. Error mapping, timeouts and the protocol stay the same. We do **not** treat EINVAL as success or retry: the order fixes the cause.

## 4. Tests

- **Regression unit test in `orch-protocol/src/bus.rs` tests:** a server that reads the line, replies at once and closes, and many `round_trip` calls in a row (e.g. 500) that must all return the reply. Run it on the old code first and note in the report whether it failed there (on macOS a tight loop should hit the race). If it cannot be made to fail before the fix, keep it anyway as a guard and say so.
- If `orch-hook` has a test seam for `send` against a socket, add the same quick-reply-and-close case there; otherwise its existing tests must stay green.
- `idle_agents_are_suspended_and_resume_on_new_work` and the whole `e2e_fake_claude` suite unchanged and green; `cargo test --workspace` green; `cargo clippy --workspace --all-targets` clean.

## 5. Out of scope

- Other e2e timeouts or test pacing.
- Making `server::handle` keep the connection open longer (the client must not depend on that).

## 6. Team

One backend task (`back`): `orch-protocol` bus + `orch-hook` + tests. No frontend work; `front` reviews the diff if `back` asks for a review.
