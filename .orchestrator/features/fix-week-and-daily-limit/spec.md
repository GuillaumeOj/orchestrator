# Fix Week And Daily Limit: spec

## Problem

The pacing indicator in the title bar (`ui/src/components/PacingIndicator.svelte`),
both the compact strip and the popover it opens, draws the week bar from
`s.governed` alone, which is the agents' share. The total week usage `s.used`
(`W`) shows only as text ("Week 56%"). Usage from the user's own sessions
(`s.interactive_used`) and from outside the app (`s.unattributed_so_far`:
claude.ai, other terminals) shows only as a muted text line in the popover.

So the bar reads 47% full while the account is really at 56%, and the user
misreads how much of the week is left. The tick on the bar is `s.ideal`, the
agents' curve, so it doesn't match the total either.

The 5h bar already uses the account-wide reading (`u.five_hour.used_pct`, which
includes outside usage). It stays as it is.

## Data (no backend change)

The core already sends everything in `app.data.usage.snapshot`
(`crates/governor/src/governor.rs`, `ForecastSnapshot`):

| field | meaning |
|---|---|
| `used` | total week usage W, % |
| `unattributed_so_far` | outside the app so far, pp |
| `interactive_used` | the user's own (interactive) usage, pp |
| `governed` | agents: `max(0, W − interactive_used − unattributed_so_far)` |
| `ideal` | the agents' curve now |
| `plan` | where W should be now: `ideal + interactive_used + unattributed_so_far` |
| `error` | `governed − ideal` = `W − plan`, pp |

## Change

### Week bar = total usage, stacked

Both week bars (compact and popover) fill up to `W` and show three segments,
left to right:

1. **outside app**: `unattributed_so_far`, muted tone
2. **you**: `interactive_used`, neutral secondary tone
3. **agents**: the rest up to `W`, in the pressure color (as today)

The agents go last so they sit on top of the usage they don't control, and the
tick lands inside their lane.

- Segment edges are cumulative and clamped: `a = min(outside, W')`,
  `b = min(outside + you, W')`, end `= W'`, with `W' = min(100, W)`. The fill
  always ends at the "Week X%" figure, and nothing overflows when the inputs
  disagree by rounding.
- **The tick moves from `ideal` to `plan`.** `plan` sits on the same total
  scale as the fill, so the gap between the fill end and the tick is the
  `(+9)` delta already in the text.
- Segments that come out at 0 width don't render (no stray 1 px slivers).
- Colors come from existing tokens in `ui/src/styles/tokens.css`. No new hex
  values. They must read in dark, light and high-contrast themes; outside and
  you differ in tone, not just hue.

### Popover legend

The line "you X% · outside app Y% · projected Z% at the reset" gets a small
color swatch before "you" and "outside app" that matches the segments, and
gains "agents N%" with its swatch, so all three segments are named. The
"projected" part stays.

### Accessibility

Each week bar gets an `aria-label` / `title` that names the parts, for example
"Week 56%: agents 47%, you 5%, outside app 4%; plan 47%".

### Text figures

The text stays as it is ("Week 56%", "agents 47%", "plan 38% (+9)"). Only the
graphics change. The agents' plan figure (`ideal`) in the text is still right
for the agents; the tick shows the total plan.

## Implementation notes

- Put the math in a pure helper in `ui/src/lib/pacing.js`, for example
  `weekSegments(s) → { outside, you, agents, tick }` in % of the bar width,
  and use it from both bars.
- `.bar > i` in `ui/src/styles/app.css` is one fill. The week bars need several
  absolutely positioned or flex segments. Keep the other `.bar` users (the 5h
  bar, the sidebar) unchanged.

## Out of scope

- The Week planner pane (`PlannerPane.svelte`) chart: it plots the agents'
  curve on purpose and already lists the totals in its table.
- Any core, protocol or estimator change.
- The 5h bar (already total).

## Acceptance

1. With the test fixture (`used 56, governed 47, interactive_used 5,
   unattributed_so_far 4, plan 47, ideal 38`), both week bars show segments of
   4 / 5 / 47 % and the tick at 47 %.
2. Clamping: `used 120` (or segments adding up past `used`) never draws past
   100 % and the fill end equals `min(100, used)`.
3. With `interactive_used = 0` and `unattributed_so_far = 0`, the bar looks as
   it does today: agents only, and the tick at `plan` (= `ideal`).
4. The text figures are unchanged, and the existing test
   `title bar figures follow J8` still passes.
5. Unit tests for `weekSegments` in `ui/test/pacing.test.js`, plus a component
   test on the rendered segment widths and tick position.
6. `bun run` lint/format (biome) and the UI test suite pass.

## Team

Frontend only: one task for `front`. `back` reviews it; no backend task.
