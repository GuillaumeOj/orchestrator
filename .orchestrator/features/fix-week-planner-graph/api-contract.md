# Fix Week Planner Graph · API contract

`planner_view` (Tauri command, `crates/orch-app/src/commands.rs`) → `PlannerView` (`crates/orch-app/src/planner.rs::build`). Only the chart's fields change. All values are % of the weekly limit.

Notation (all from `gov.snapshot(now)` = `s` and `gov.curve()` = `c`, as `build` has them today):
- `ideal(t) = c.fraction(t) × budget` (unchanged, the agents' ideal curve)
- `other_now = s.interactive_used + s.unattributed_so_far` (= `s.plan − s.ideal`)
- `h_now = c.hours_at(now)`, `h_left = max(c.hours_left(now), 1e-9)` (weighted hours)
- `wall_left = max(c.wall_hours_left(now), 1e-9)`, `wall(t) = wall hours from now to t`

## `actual: Vec<HistoryPoint>` (changed meaning, same type)

`{ t_ms, used, governed, five_hour }`, oldest first:
1. `history.thinned(400)` as today, then
2. a last point at `now`: `{ t_ms: now, used: s.used, governed: s.governed, five_hour: s.five_hour }` (replaces it if the last sample has `t_ms ≥ now`).
3. Then, in order: `used = max(used, previous used)`, `governed = max(governed, previous governed)`, then `governed = min(governed, used)`.

The last point's `used` may then be above `s.used` only when an estimate was corrected down since; the UI tag still shows `s.used`. The history kept in memory and in the store is not changed (the running max is a view of it).

## `curve: Vec<CurvePoint>`

Same sampling as today (hourly from week start to reset, plus `now`). Fields:

| Field | Type | Value |
|---|---|---|
| `t_ms` | `i64` | unchanged |
| `plan_total` | `f64` | `t ≤ now`: `ideal(t) + other(t)`, where `other(t) = max(0, used − governed)` of the latest **raw** history sample at or before `t` (0 before the first sample; `other_now` at `now`). `t > now`: `ideal(t) + other_now + (s.unattributed_forecast − s.unattributed_so_far) × min(1, wall(t) / wall_left)`. At `now` it equals `s.plan` (± float). |
| `forecast_total` | `Option<f64>` | `t ≥ now`: `min(110, s.used + (s.projected − s.used) × clamp((c.hours_at(t) − h_now) / h_left, 0, 1))`; else `None`. |
| `with_governor_total` | `Option<f64>` | `t ≥ now`: `max(s.used, plan_total(t) + s.error × (1 − clamp((c.hours_at(t) − h_now) / h_left, 0, 1)))`; else `None`. |

`ideal`, `forecast`, `with_governor` (agents-only basis) are removed from `CurvePoint` if no other reader is found (`grep` the UI and crates); if one is, keep them and say so in the report.

## Unchanged

`snapshot` (all of `ForecastSnapshot`), `reserve_pct`, `working`, `allocations`, `queue`, `log`, `next`, `off_hours`, `week_start_ms`, `reset_at_ms`, `now_ms`.

## Tests (`planner.rs`)

- Dipping history (e.g. used 30 → 34 → 31 → 36, governed 20 → 25 → 18 → 26) → `actual` non-decreasing on both fields, `governed ≤ used`, last point at `now`.
- At `now`: `plan_total ≈ s.plan`, `forecast_total ≈ s.used`, `with_governor_total ≈ max(s.used, s.plan + s.error)`; at the reset: `forecast_total ≈ min(110, s.projected)`, `with_governor_total ≈ max(s.used, plan_total)`.
- The existing `curve_matches_the_snapshot_at_now` test is updated to the new fields.
