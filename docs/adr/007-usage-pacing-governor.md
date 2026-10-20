# ADR-007 — Usage pacing: Governor control law (v1)

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-17 (control law added after review)

## Context
A Max plan has a 5-hour window and a weekly window, and possibly a separate weekly limit for some models. Claude Code's statusline exposes used % and reset times for the 5h and weekly windows (ADR-002). Goal: never get locked out during working hours, and use ≥ 85 % of the week by the reset.

## Admission points (what the Governor really controls)
1. **Turn start**: every autonomous turn (a task, a message, a "continue") is delivered by the app, so it can be held in the queue.
2. **Each tool call**: a PreToolUse hook asks for admission. If the Governor holds the agent, the hook waits up to a few seconds, then denies with "paused by governor" and the turn ends at that boundary. **Hook timeouts fail open** (P0: a hook past its `timeout` was dropped and the tool ran), so `orch-hook` enforces its own deadline well below the configured timeout (600 s) and always answers. Parallel tool calls run their hooks concurrently, and each one is admitted separately.
3. **Watchdog**: a turn exceeding its ceiling (usage or duration) is interrupted at the next tool boundary.
4. **Never blocked**: what the user types in a pane. Built-in subagents run inside their agent's turn and are counted against it.

## Measurements
- `W` = weekly used %: **observed** from the statusline when it is fresh (< 15 min). Otherwise it is **estimated** as `W_last + cost_since / k + margin`, where `k` is one scalar ("cost estimate per 1 % of the week") updated by an exponentially weighted average whenever a fresh observed value arrives. The margin is 10–15 pp and shrinks as confidence grows.
- `F` = 5h used %, observed the same way.
- Observed values come from the statusline (`rate_limits`, integer %, `resets_at` in epoch s). They are missing until a session's first API response, and idle agents refresh them through `statusLine.refreshInterval`.
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
- Every queued item shows why, its estimated start, and "Run now" with its estimated cost.
- **Shadow mode** is on for the first week.
- The **simulator** (SPEC FR-820, NFR-9) replays weeks against the policy; it is the acceptance test.

## Consequences
- Pacing that can be predicted and tested, with honest limits: queuing *defers* usage, it doesn't reduce it. Mostly-interactive weeks leave the Governor little to act on, and the planner says so.
