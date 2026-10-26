# ADR-007 — Usage pacing: Governor control law (v1)

- Status: Proposed · Date: 2026-10-17 · Revised: 2026-10-26 (state across restarts, same forecast after a restart)

## Context
A Max plan has a 5-hour window and a weekly window, and possibly a separate weekly limit for some models. Claude Code's statusline exposes used % and reset times for the 5h and weekly windows (ADR-002). Goal: never get locked out during working hours, and use ≥ 85 % of the week by the reset.

## Admission points (what the Governor really controls)
1. **Turn start**: autonomous turns the app delivers (a task, a message, a "continue") can be held in the queue. Not every turn comes from the app: async subagents and background shells start a new turn by themselves when they finish (`<task-notification>`, P0 recovery spike). Those are counted against the agent's allocation, still pass point 2, and can be disallowed per role (SPEC FR-410).
2. **Each tool call**: a PreToolUse hook asks for admission. If the Governor holds the agent, the hook waits up to a few seconds, then denies with "paused by governor" and the turn ends at that boundary. Hook timeouts, crashes and a missing hook binary all **fail open** (the tool runs), so `orch-hook` always answers itself and **fails closed**: if the core is unreachable or misses the deadline, it denies (SPEC FR-806). Parallel tool calls run their hooks concurrently, and each one is admitted separately. While holding, the core watches the hook's connection: Esc in the pane kills the hook without any event.
3. **Watchdog**: a turn exceeding its ceiling (usage or duration) is interrupted at the next tool boundary.
4. **Never blocked**: what the user types in a pane. Built-in subagents run inside their agent's turn and are counted against it.

## Measurements
- `W` = weekly used %: **observed** from the statusline when it is fresh (< 15 min). Otherwise it is **estimated** as `W_last + cost_since / k + margin`, where `k` is one scalar ("cost estimate per 1 % of the week") updated by an exponentially weighted average whenever a fresh observed value arrives. The margin is 10–15 pp and shrinks as confidence grows; it is never more than the extrapolated delta, except before the week's first reading when the Governor did not see the reset.
- `F` = 5h used %, observed the same way.
- Observed values come from the statusline (`rate_limits`, integer %, `resets_at` in epoch s). They are missing until a session's first API response. Each agent's statusline is a **copy of its own last API response**, so `W` and `F` use the most recent sample across all agents for each window, and a window missing from a response is unknown, not 0. A plan-limit 429 shows as StopFailure `rate_limit` plus `used_percentage: 100`; the CLI then arms its own auto-continue at the reset, which the Governor cancels and reschedules by default (SPEC J9).
- **Resume cost** uses the statusline `prompt_cache` (`expires_at`, `recache_tokens_if_cold`): resuming before expiry is nearly free (1 h TTL observed).
- `U` = unattributed rate `u` (pp per wall hour), fitted together with `k` from the observed deltas (`ΔW ≈ ΔA / k + u · Δt`, exponentially weighted). The weekly reset is a reading of W = 0: the week's first reading is calibrated against it on every path (the app running at the reset, starting later in the week, realigning the plan week on a reading, or replaying the week's samples after a restart), so all of them give the same `k`, `u` and forecast (decision 2026-10-26).
- **Forecast rate** `u' = (u·h + u0·H0) / (h + H0)`: the fitted rate shrunk toward a prior `u0` by the hours `h` of the week observed so far (`H0` = 48 h by default, `u0` = 0 until it is seeded from earlier weeks). One heavy day of outside use early in the week is not extrapolated over the whole week; the fit takes over as the week goes on. All of the week's estimates use `u'` (decision 2026-10-26: a restart on a reset day with 17.6 % used before the app forecast 183 % outside the app from one 14.5 h interval).

## Ideal curve
`ideal(t) = H(t) / H_week × (100 − reserve − U_forecast)`, where `H` counts **weighted working hours** from the calendar (off-hours count only if an off-hours allowance is enabled). Error on the **governed share**: `G = W − R_used − U_actual` (total minus the reserve used by interactive work minus the unattributed usage observed so far), `e = G − ideal(t)` in pp, and `U_forecast = U_actual + u' × wall hours left`. Interactive and outside-app usage therefore never push the Governor into Amber/Red by themselves (decision 2026-10-21). Critical agent work also draws on the reserve (admission rule) but is the agents' own usage, so it stays in G. The exhaustion rule works on the **total** W projected at the reset: `P = W + burn × H_left + u × off-hours left`, where `burn` is the exponentially weighted burn rate per weighted hour (so it follows the curve: half-weight blocks, a sprint borrowing from later days) and `u'` covers outside usage in the hours when agents are stopped and the burn is not measured. The burn average is bias-corrected: the weight it has not measured yet (after the reset or a restart) goes to the **on-plan pace**, `((budget − ideal) + u' × working wall hours left) / H_left` with `budget = 100 − reserve − U_forecast`, not to the full `100 − reserve`. Outside usage that the plan has already absorbed (by shrinking the agents' budget) therefore does not project the week over the limit by itself (decision 2026-10-26). Heavy interactive or outside use can therefore still trigger Red. P is in pp rather than a time so that the rule keeps its hysteresis at the end of the week; an `eta < reset − 4 h` rule lost it there, because once only off-hours remain both its thresholds mean "runs out before working time ends", and it flipped Red/Green every 30 min (decision 2026-10-26). `eta`, the time the total reaches 100 % along the curve, is still shown. The title bar shows W as "Week" and G against the plan as "agents".

## Pressure states (hysteresis, minimum dwell 30 min)
| State | Enter | Exit | Live autonomous agents |
|---|---|---|---|
| Green | default | — | `base` (e.g. 3) |
| Amber | e > +3 pp | e < +1 pp | `ceil(base/2)` |
| Red | e > +8 pp or P > 98 % | e < +5 pp and P < 96 % | 1, only tier ≥ normal |
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
- **Shadow mode** is on for the first week. In shadow mode every action that would change admission (state changes, the 5h guard, deferrals, `reserve_exhausted`) is logged with mode *shadow*; alerts and limit records stay as they are.
- The **simulator** (SPEC FR-820, NFR-9) replays weeks against the policy; it is the acceptance test.

## State across restarts
- The Governor stays a pure library with its state in memory. The core stores the active plan, the policy, every new statusline sample with the **cost class** it had (interactive / critical / autonomous) and its allocation, the recent actions, the queue (as a view), forecast snapshots and the calibration.
- At start the core builds a new Governor on the active plan (if its week is still running) and **replays** this week's samples with their stored class (`Governor::replay`), so W, the governed share, per-allocation usage and the calibration come back. A plain re-ingest would count past agent work as interactive.
- The replayed Governor and the live one agree for the same samples: both calibrate the week's first reading against W = 0 at the reset (see Measurements). Two differences remain: the burn is measured between control-loop ticks, which a replay does not have, so after a restart it starts again from the on-plan pace; and the calibration is not carried across weeks by a restart (the live Governor keeps `k` and `u` from earlier weeks). A later step persists the calibrator and seeds `u0` from last week's observed outside usage.
- The queue is not restored: turns that waited in the old queue are asked again by the reconciler. Action ids restart at each core start.

## Consequences
- Pacing that can be predicted and tested, with honest limits: queuing *defers* usage, it doesn't reduce it. Mostly-interactive weeks leave the Governor little to act on, and the planner says so.
