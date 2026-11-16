# Fix Overwatch trust folder — API contract

Only one change crosses the core/UI boundary.

## `now.overwatch` (StatusView)

Unchanged shape: `{status, events, why, since_ms, agent, has_session}`.

New value of `why`, with `status: "paused"`:

| why | when | status |
|---|---|---|
| `"waiting for folder trust"` | the Overwatch's trust dialog is still on screen after the 10 s auto-answer window, or the auto-answer gave up (spec §3.4) | `paused` |

- `has_session` is `true` in that state (the session was spawned today), so the
  header status and the strip's **Open session** open it (existing
  `canOpenSession`).
- It clears (back to `waking` / `working` / `asleep`) at the session's first
  SessionStart.
- Precedence: `off` > `down` > `paused` (Pause all) > `paused` (limit reached)
  > `paused` (folder trust) > working states. Only one `why` at a time.

The constant lives in `agent-runtime/src/overwatch/mod.rs` `why::TRUST`; the UI
compares the string (like the other reasons) in `ui/src/lib/overwatch.js`, as
an exported constant `TRUST_WHY`.

## Events (core log, no UI change)

- `overwatch.trust_answered {dir}`
- `overwatch.trust_failed {dir, why}`

## Decisions / Inbox

The `trust-overwatch` decision and Inbox card keep their shape; they only
appear in the fallback, with the Overwatch copy of spec §3.4.
