# Fix Week Planner Graph · UX brief

No design pass: the Week card keeps its layout, sizes and tokens (`PlannerPane.svelte`). This brief is what `ui` reviews T-2 against.

## The question the chart answers

"How much of this week's limit have we used so far, and are we on plan?" One glance: the solid line climbs from 0 at the week start to today's usage; the dashed plan line is where it should be.

## Marks (back to front)

1. Working-hours shading, off-hours hatching: unchanged.
2. Reserve band at the top (`100 − reserve` → 100), `.reserve` as today. Label, right-aligned above it: `reserve {r}% for you`.
3. **Week used**: filled area + solid line, the main mark. Keeps today's `.fill` / `.actual` styles (pacing red, 2px, 15% fill).
4. **Agents**: filled area only (no stroke), under Week used, drawn on top of its fill so it reads as "the agents' part" of it. Accent colour (`--color-accent-default`), ~20% opacity. No new token.
5. **Plan**: dashed, `.ideal` style today (text-secondary, 6/4 dash).
6. **Forecast**: dashed, `.fc` today (pacing red, dashed), from now.
7. **W/ governor**: dashed, `.gov` today (accent, dashed), from now.
8. Now marker + tag: `now 14:02 · week 42% (plan 38%, +4 pp) · agents 30%`. The `⚠ empty …` tag is unchanged.

## Legend (card header, left → right)

`Week used` (solid red swatch) · `Agents` (accent block swatch) · `Plan` (dashed grey) · `Forecast` (dashed red) · `W/ governor` (dashed accent).

## Words

- "Week used" for the total, "agents" for their part: same words as the title bar and the allocations footnote ("Agents 30% vs plan … · total …").
- Chart `aria-label`: "Week usage so far against the plan".
