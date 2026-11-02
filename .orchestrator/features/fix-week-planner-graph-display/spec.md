# Fix Week Planner Graph Display: spec

## Problem

The user reads the Week planner's chart (`ui/src/panes/PlannerPane.svelte`, the
"Week" card) as "the current hours are shaded at 100%", even though last week's
feature (`fix-week-and-daily-limit`) made the title bar show the real weekly
usage.

There are two causes in the chart:

1. **The working-hours shading is full height.** Each working block is a
   `<rect class="work">` from `pad.t` (the 110% line) down to 0%, filled with
   `--color-surface-raised` at 50% opacity. On the dark theme (`#292e42` on the
   card) these are grey columns from the bottom to the top of the plot. The
   current day's block reads as a bar at 100%. It is only a background meaning
   "working time", but it looks like a value.
2. **The chart doesn't show the real usage.** The only filled area
   (`.fill`, red at 15% opacity) is under the agents' share `G`
   (`HistoryPoint.governed`). The total weekly usage `W` (`used`), which is
   what the title bar now shows and what counts toward the limit, is only in
   the "now" tag text. With little agent work the red fill is a thin smear at
   the bottom and the grey columns are all you see. `fix-week-and-daily-limit`
   left the chart out of scope on purpose. This feature brings it in line.

The data needed is already there. `PlannerView.actual` (`HistoryPoint`) carries
`used` (W) and `governed` (G) for every sample, and `snapshot.used` is W now.
**No backend change.**

## Change (frontend only, `PlannerPane.svelte`)

### 1. Working hours become a band on the time axis

- Drop the full-height `.work` rects from the plot area.
- Draw working blocks as a thin band (about 4–6 px) in the bottom gutter, just
  under the 0% line and above the day labels (`pad.b` grows if needed). It is a
  timeline of when the agents work. Use an existing token with enough contrast
  in the dark, light and high-contrast themes. No new hex values.
- Off-hours hatching (`offBands`, only when the plan allows off-hours work) moves
  to the same band, so the band reads "working | off-hours | nothing". The
  "off-hours" label and the "☾ N queued" tag stay, positioned near the band.
- Update the card subtitle: "working hours shaded · off-hours hatched" becomes
  "band: working hours · off-hours hatched" (the hatched part only with an
  allowance, as today).

Nothing in the plot area spans the full height any more except the "now" line
and the grid.

### 2. Real usage as the filled area

- **Total area (W):** fill from 0 up to `used` over `actualPts` (history `used`,
  then `[now_ms, s.used]`). Use a muted tone, the same family as the
  outside/you segments of the title bar week bar, so it reads as "everything
  counted toward the limit".
- **Agents area (G):** the existing red fill under `governed`, drawn on top of
  the total area. `G ≤ W` always, so the band between the two edges is you plus
  outside the app.
- **Total line:** a thin solid line on the W edge, so the real usage can be read
  against the 0–100% grid and the reserve band. The red agents line stays as it
  is.
- Legend: add "total" (its swatch) before "agents". The existing entries (ideal,
  forecast, w/ governor) stay. They are on the agents' scale and are compared
  with the agents' line, as today.
- The "now" tag stays. It already prints "agents X% (plan Y%, ±pp) · total Z%".
  Anchor it above the higher of the two edges at now (`max(s.used, s.governed)`
  = `s.used`), so it doesn't overlap the total line.
- The chart's `aria-label` names both figures, for example "Week usage: total
  56%, agents 47% against the ideal 38%".

### 3. Clamping

`y()` already clamps to 0–110. If a history point has `governed > used`
(rounding), the agents area is clamped to `used` so it never pokes out above the
total area.

## Out of scope

- The governor's math, the estimator, the history sampling and the store
  seeding (`core.rs::past_week`, `planner.rs`).
- A total-scale forecast curve (projected W to the reset). The 5h card and the
  title bar already show `projected`.
- The other cards of the planner (5h window, allocations, next action, log).

## Acceptance

1. No full-height rect in the Week chart's plot area: working blocks render as
   a band in the bottom gutter, one rect per `v.working` block, and off-hours
   hatching in the same band only when `offHoursAllowed(off)`.
2. With a fixture where history `used` > `governed` (for example the last point
   `used 56, governed 47`), the chart has a total area whose top edge at now is
   at `y(56)` and an agents area whose top edge at now is at `y(47)`.
3. `governed > used` on a point never draws the agents area above the total
   area.
4. With `used == governed` everywhere, the total area is hidden behind the
   agents area (it looks like today without the grey columns).
5. Colors come from `ui/src/styles/tokens.css` and read in dark, light and
   high-contrast themes.
6. Component tests in `ui/test/pacing.test.js` (the planner describe block)
   cover 1–4. The existing planner tests still pass (the off-hours band with the
   queued count, O and R).
7. Biome lint/format and the UI test suite pass.

## Team

Frontend only: one task for `front`. `back` reviews it. No backend task.
