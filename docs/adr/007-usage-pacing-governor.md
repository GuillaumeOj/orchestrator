# ADR-007 — Usage pacing: Governor control law (v1)

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (control law added after review)

## Context
A Max plan has a 5-hour window and a weekly window, and possibly a separate weekly limit for some models. Claude Code's statusline exposes used % and reset times for the 5h and weekly windows (ADR-002). Goal: never get locked out during working hours, and use ≥ 85 % of the week by the reset.

## Admission points (what the Governor really controls)
1. **Turn start**: autonomous turns the app delivers (a task, a message, a "continue") can be held in the queue. Not every turn comes from the app: async subagents and background shells start a new turn by themselves when they finish (`<task-notification>`, P0 recovery spike). Those are counted against the agent's allocation, still pass point 2, and can be disallowed per role (SPEC FR-410).
2. **Each tool call**: a PreToolUse hook asks for admission. If the Governor holds the agent, the hook waits up to a few seconds, then denies with "paused by governor" and the turn ends at that boundary. Hook timeouts, crashes and a missing hook binary all **fail open** (the tool runs), so `orch-hook` always answers itself and **fails closed**: if the core is unreachable or misses the deadline, it denies (SPEC FR-806). Parallel tool calls run their hooks concurrently, and each one is admitted separately. While holding, the core watches the hook's connection: Esc in the pane kills the hook without any event.
3. **Watchdog**: a turn exceeding its ceiling (usage or duration) is interrupted at the next tool boundary.
4. **Never blocked**: what the user types in a pane. Built-in subagents run inside their agent's turn and are counted against it.

## Measurements
- `W` = weekly used %: **observed** from the statusline when it is fresh (< 15 min). Otherwise it is **estimated** as `W_last + cost_since / k + margin`, where `k` is one scalar ("cost estimate per 1 % of the week") updated by an exponentially weighted average whenever a fresh observed value arrives. The margin is 10–15 pp and shrinks as confidence grows.
- `F` = 5h used %, observed the same way.
- Observed values come from the statusline (`rate_limits`, integer %, `resets_at` in epoch s). They are missing until a session's first API response. Each agent's statusline is a **copy of its own last API response**, so `W` and `F` use the most recent sample across all agents for each window, and a window missing from a response is unknown, not 0. A plan-limit 429 shows as StopFailure `rate_limit` plus `used_percentage: 100`; the CLI then arms its own auto-continue at the reset, which the Governor cancels and reschedules by default (SPEC J9).
- **Resume cost** uses the statusline `prompt_cache` (`expires_at`, `recache_tokens_if_cold`): resuming before expiry is nearly free (1 h TTL observed).
- `U` = unattributed rate: the exponentially weighted average of (observed weekly delta − the app's attributed delta).

## Ideal curve
`ideal(t) = H(t) / H_week × (100 − reserve − U_forecast)`, where `H` counts **weighted working hours** from the calendar (off-hours count only if an off-hours allowance is enabled). Error `e = W − ideal(t)` in pp. `eta` = projected time when W reaches 100 % at the current exponentially weighted burn rate.

## Pressure states (hysteresis, minimum dwell 30 min)
| State | Enter | Exit | Live autonomous agents |
|---|---|---|---|
| Green | default | — | `base` (e.g. 3) |
| Amber | e > +3 pp | e < +1 pp | `ceil(base/2)` |
| Red | e > +8 pp or eta < reset − 4 h | e < +5 pp and eta ≥ reset | 1, only tier ≥ normal |
| Catch-up | e < −10 pp and < 24 working hours left | e > −5 pp | `base + 2`, queue released |

All thresholds live in `PacingPolicy` and can be adjusted.

**5h guard** (independent of the weekly state):
- If the projected `F` at the 5h reset is > 85 %, the live cap is 1.
- If `F` > 85 %, only interactive and critical work is admitted.
- Margin for the overrun: Claude Code may finish a step in progress after the 5h limit is hit, and that capped overrun counts toward the weekly limit (Anthropic support, 2026-10-20). The 85 % threshold leaves room for it.

## Admission rule
```
admit(item):
  if item.interactive or item.tier == critical: admit        # draws the reserve first; warn if reserve < 0
  if state == Red and item.tier <= normal: queue; propose "defer after reset"
  if live_autonomous >= cap(state, 5h guard): queue
  else admit
next from queue = weighted fair queuing over allocation weights,
                  strict for critical, aging +1 tier per 2 h waited
```
Unused weight is redistributed. Nothing rolls over to the next week. Sprint mode borrows from the later days: the curve goes up for today and down for the remaining days.

## Actions in v1
Live cap per state, queue, pause at turn boundary, defer after reset (asked, with a default and a 20-min timeout), 5h guard, watchdog.
- **Deferred to later phases**: 5h peak smoothing and per-model estimator weights.
- **Never automatic**: model downgrade (opt-in per role, at task boundaries; it breaks the prompt cache and risks quality).
- **Not a lever**: compaction.

## Transparency & safety
- Every action is a `GovernorAction` record with its reason, expected effect and undo.
- Every queued item shows why, its estimated start, and "Run now" (or "Start now" if it never ran) with its estimated cost in the window that binds it (5h % under the 5h guard, week pp otherwise).
- **Shadow mode** is on for the first week.
- The **simulator** (SPEC FR-820, NFR-9) replays weeks against the policy; it is the acceptance test.

## Consequences
- Pacing that can be predicted and tested, with honest limits: queuing *defers* usage, it doesn't reduce it. Mostly-interactive weeks leave the Governor little to act on, and the planner says so.
