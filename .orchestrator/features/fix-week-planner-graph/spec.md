# Fix Week Planner Graph · Feature spec

> Feature `fix-week-planner-graph` · Integration branch `orch/fix-week-planner-graph/integration` · core (`orch-app` planner view) + UI (`PlannerPane`) · no design pass (ui reviews the chart)

## 1. Problem

The user reads the Week chart in the Week planner like this: **100% is the week's limit**, and each point of the solid line is **how much of the week has been used so far**. So the line can only go up, and it ends at the week's usage now.

The chart does not show that today, so its line reads like an "instant usage" of the agents:

- The solid line ("agents") is the **governed share G**: `W − your own usage − outside-app usage` (`crates/governor/src/governor.rs` `metrics`). It is not the week's usage. It sits well below the "total" figure in the same tag, and the gap widens every time you or claude.ai use the plan.
- G is cumulative in theory, but its drawing **goes down**:
  - while the reading is stale, W carries an estimate margin (up to `margin_max_pp`) that falls off when a fresh reading arrives, so the line drops back (a sawtooth);
  - G is agent cost ÷ the calibrated cost-per-% `k`, so a new calibration rescales the latest points but not the earlier ones (steps up or down).
- The ideal, forecast and "w/ governor" dashed lines are on the same agents-only basis, so none of them compare with the week's real usage.

**Goal:** the chart shows the week's usage so far against the week's limit: a line that only goes up, with the agents' part of it shown inside, and plan / forecast lines on the same basis.

## 2. What the chart shows after the fix

Y axis unchanged: % of the weekly limit, 0–100% (110% headroom as today). X axis unchanged: week start → reset.

| Mark | Meaning | Source |
|---|---|---|
| **Week used**: solid line + fill (main) | Total weekly usage `W` so far, at each point of the week. Never goes down. Ends at `now` = `snapshot.used`. | `actual[].used` |
| **Agents**: second, lighter filled area under it | The agents' part of that usage (`G`). Never goes down, never above Week used. The gap between the two = you + outside the app. | `actual[].governed` |
| **Plan**: dashed | Where the week's usage should be if the agents are on plan: the agents' ideal curve + your usage + outside-app usage (so far, then its forecast). At `now` it equals `snapshot.plan`. | `curve[].plan_total` |
| **Forecast**: dashed, from now | The week's usage projected at the current burn, ending at `snapshot.projected` at the reset. | `curve[].forecast_total` |
| **W/ governor**: dashed, from now | The week's usage if the Governor closes today's error by the reset. Ends on the plan line. | `curve[].with_governor_total` |
| Reserve band at the top | The reserve kept for your own work: `100 − reserve_pct` → 100. | `reserve_pct` |
| Now marker + tag | `now 14:02 · week 42% (plan 38%, +4 pp) · agents 30%` | `snapshot.used`, `snapshot.plan`, `snapshot.error`, `snapshot.governed` |

The error stays the same number as today (`G − ideal` = `W − plan`), so the pressure states, the title bar and the allocations table do not change.

"Never goes down": W only grows inside a week. A drop in the samples is the correction of an estimate (margin, calibration), not usage given back, so the drawn line holds its level until the usage passes it again (a running max over the week, see `api-contract.md`).

## 3. Scope

**Core (`crates/orch-app/src/planner.rs`, `back`)**: the view sends the chart data on the week-usage basis (see `api-contract.md`):
- `actual`: the week's samples + a last point at `now` from the snapshot, with `used` and `governed` made non-decreasing and `governed ≤ used`.
- `curve`: `plan_total`, `forecast_total`, `with_governor_total` per point; the agents-only fields (`ideal`, `forecast`, `with_governor`) are removed if nothing else reads them.
- Unit tests in `planner.rs`.

**UI (`ui/src/panes/PlannerPane.svelte`, `front`)**: draw the marks of §2 from those fields, new legend, new now tag, reserve band label, `aria-label` "Week usage so far against the plan". The chart stops appending its own `now` point. Tests in `ui/test/pacing.test.js` / `ui/test/projects.test.js` (they already render `PlannerPane`).

**Out of scope**: the Governor's maths (`governor` crate: G, error, pressure states, burn), the stored forecast snapshots, the 5h card, the allocations table, the weekly report.

## 4. Acceptance

1. With history samples that dip (a margin falling off, a recalibration), the drawn Week used and Agents lines never go down, and Agents is never above Week used.
2. The Week used line ends at `now` exactly on `snapshot.used`; the plan line passes through `snapshot.plan` at `now`; the forecast ends at `snapshot.projected` at the reset (capped at 110); the w/ governor line ends on the plan line at the reset.
3. The now tag reads `week X% (plan Y%, ±Z pp) · agents A%` with the snapshot's figures.
4. Legend: Week used, Agents, Plan, Forecast, W/ governor. The reserve band is labelled `reserve {r}% for you`.
5. `cargo test -p orch-app`, `cargo clippy`, `bun run test`, `bun run check`, `bun run lint` pass.

## 5. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Planner view on the week-usage basis | back | `planner.rs` only, contract in `api-contract.md` |
| T-2 Week chart draws the week's usage | front | depends on T-1; `PlannerPane.svelte` + tests; review by ui |

No UX/UI design task: the chart keeps its layout and tokens; `ui` reviews T-2 against `ux-brief.md`.
