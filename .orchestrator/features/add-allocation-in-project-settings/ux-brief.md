# Add Allocation In Project's Settings: UX brief

Context: `spec.md`. Existing patterns to follow: Project settings › Task backend
(`docs/design/img/04-project-linear.png`, `-states.png`), the Week planner's
plan editor (`06-j8-1-plan.png`, `04-week-planner*.png`), keyboard notation of
SPEC §9.7.

## Who and when

The user has several projects and a weekly plan. They registered a new project
(or never gave one a share) and see its agents counted under "other sessions"
in the Week planner. They open the project's settings (`⌘+K` "{name} settings"
→ Allocation, or the left nav) to give it a share of the week.

## Flows

**A. Add a project to the plan**
Allocation section, state *Not in the plan* → "Add to the weekly plan" → the
row appears pre-filled (weight, tier normal, no cap), share shown → adjust →
`⌘+⏎` Save → toast; the state becomes *In the plan*.

**B. Change the share**
State *In the plan* → edit weight (share updates live), tier, hard cap → Save /
`esc` Cancel (back to saved values). Save disabled until something changed.

**C. Remove from the plan**
State *In the plan* → "Remove from the plan" → the form shows what will happen
("agents charged to other sessions from the save on") → Save. No modal: it is
reversible by adding it back.

**D. No plan yet**
State *No plan* → the proposed even split (read only) and "Set up the weekly
plan" (opens the Week planner's Edit plan, `g` then `e`).

**E. Plan editor**
Week planner › Edit plan: projects not in the plan are listed under the plan's
allocations, muted, weight 0, "not in plan". Typing a weight > 0 makes the row
normal; Save adds them.

## States to design

1. Loading (skeleton, like Task backend).
2. No plan confirmed.
3. Not in the plan (charged to other sessions; Add).
4. In the plan, saved (weight, share, tier, cap, used this week, others table).
5. In the plan, edited (dirty: Save enabled, share changed).
6. In the plan with a hard cap close to / at the cap (amber / red wording, same
   tokens as the Week planner's status).
7. Pending removal (dirty).
8. Validation error (cap outside 0–100, negative weight) inline on the field.
9. Load or save error (box, like the sync error box).
10. Plan editor rows "not in plan" (muted) next to normal rows.

## Content

- Lead line: "{name}'s share of the week. Agents of {name} are charged to it;
  the rest of the plan is in the Week planner."
- Not in plan: "{name} isn't in this week's plan. Its agents are charged to
  other sessions."
- Share: "{share}% of the agents' week"; used: "used {x} pp this week".
- Others table title: "Rest of the plan".
- Keyboard hints line: `⌘+⏎` save · `esc` cancel · `a` add · `g` week planner.
  Single keys only when no field has focus, like Task backend.

## Constraints

- Same layout as the other sections: nav on the left, `h2` + lead, `.line`
  label/field rows, `.foot` buttons. No new components unless needed; reuse
  the tier select and number fields of the plan editor.
- Tokens only (`ui/src/styles/tokens.css`); readable in dark, light, high
  contrast.
- Width: works at 1280 and 1512 like the other settings frames.

## Hand-off to ui

Annotated flows and state list above (refine them in this file), then ui draws
the frames: **04 Project settings · Allocation · 1512** + **states** board, and
the plan editor row variant, exported as PNG to
`.orchestrator/features/add-allocation-in-project-settings/design/`.
