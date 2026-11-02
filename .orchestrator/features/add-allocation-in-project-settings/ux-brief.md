# Add Allocation In Project's Settings: UX brief (hand-off to ui)

Context: `spec.md`, data in `api-contract.md`. Patterns followed: Project
settings › Task backend (`docs/design/img/04-project-linear.png`, `-states.png`),
the Week planner's Allocations table (`04-week-planner-p3.png`) and Edit plan
(`06-j8-1-plan.png`, `PlanEditor.svelte`), keyboard notation of SPEC §9.7.
Code to match: `ProjectSettingsPane.svelte` (nav, `h2` + lead, `.line` rows,
`.foot` buttons, `onKey`), `PlanEditor.svelte` (tier select, number fields).

## Who and when

The user has several projects and a weekly plan. A project was registered after
the plan was saved (or never got a share): its agents show under "other
sessions" in the Week planner. They open `{name} settings` (`⌘+K`, or the left
nav) › Allocation to give it a share of the week, see what that share is
relative to, and tune or remove it later.

## Layout (one screen, top to bottom)

Nav on the left, "Allocation" selected (nav entry already exists). Right side:

1. `h2` **Allocation** + lead line (see Copy).
2. **This project** block (`.line` rows, label column then field):
   1. `Weight` number field (min 0, step 1) · live text `{share}% of the plan`
   2. `Tier` select: critical / high / normal / low
   3. `Hard cap (%)` number field, placeholder `none` · hint `stops {name}'s agents for the week at this % of the week`
   4. `This week` read only: `used {x} pp` and, with a cap, a thin bar with the cap tick + status words (state 6). Bar uses the Week planner's Used/plan bar style.
3. **Rest of the plan** table, read only: Allocation · Weight · Share · Tier ·
   Used. This project's row is shown in its place in the table, highlighted
   (so the share reads against the others; its place is `own_index`) and follows the typed weight live;
   "other sessions" last. Link `Edit the plan in the Week planner` + `g`.
4. Remove link (ghost, `danger` text): `Remove from the plan` + `x`.
5. `.foot`: **Save** `⌘+⏎` (primary) · Cancel `esc` · then the keys line.

Field order is fixed: Weight → Tier → Hard cap (same as the plan editor
columns). Tab order follows it, then Remove, Save, Cancel.

## Flows

**A. Add a project to the plan.** State 3 → `Add to the weekly plan` (`a`) →
block 2 appears pre-filled (weight = `proposed_weight`, tier normal, no cap),
weight field focused and selected, share shown, own row appears in the table →
adjust → `⌘+⏎` → toast, state 4. Nothing is saved before Save; `esc` returns to
state 3.

**B. Change the share.** State 4 → edit weight / tier / cap (share and the
table row update as typed) → state 5 → Save, or `esc` back to the saved values.
Save is disabled while nothing changed or while a field is invalid.

**C. Remove from the plan.** State 4 → `Remove from the plan` (`x`) → state 7
(no modal: reversible until Save): fields disabled and struck, notice "From
the save on, {name}'s agents are charged to other sessions." with
`Keep in the plan` (`esc` does the same) → Save applies. A weight of 0 typed in
the field is the same as remove: the notice appears and the state is 7, but
the Weight field stays enabled (the other fields are disabled), so typing `0`
on the way to `0.5` or `5` does not drop the focus; a weight above 0 leaves
state 7. `Keep in the plan` restores the weight from before the removal.

**D. No plan yet.** State 2 → read only proposed even split table, button
`Set up the weekly plan` (`g`) opens Week planner › Edit plan. No Save, no
fields. The section confirms nothing by itself.

**E. Plan editor.** Week planner › Edit plan: after the plan's own rows and
before "other sessions", every project not in the plan is listed, one row each
(state 10). It is muted, weight 0, Share `0%`, tier normal, cap `none`, plus
the tag `not in plan` after the name. Typing a weight > 0 turns the row normal
(tag removed, share computed, label is the project name); back to 0 mutes it
again. Save adds the rows with a weight. The info line under the table gets one
more sentence (Copy).

## States (10)

| # | State | What it shows |
|---|---|---|
| 1 | Loading | Lead line, three skeleton rows in block 2, skeleton table (4 rows). Save disabled. Same skeleton as Task backend. |
| 2 | No plan confirmed | Lead, notice "No weekly plan yet. The core proposes an even split; nothing is applied until you set it up." Read only split table (project row highlighted, shares equal), button `Set up the weekly plan` `g`. No foot. |
| 3 | Not in the plan | Lead, notice "{name} isn't in this week's plan. Its agents are charged to other sessions." Button `Add to the weekly plan` `a`. Table of the plan below, "other sessions" row highlighted with its used pp. Save disabled. |
| 4 | In the plan, saved | Block 2 filled, share, `used {x} pp`, table, Remove, Save disabled, Cancel disabled. |
| 5 | In the plan, edited | As 4, changed fields show a dot marker, share and own table row follow the typed weight, Save enabled, Cancel enabled. Also the state right after Add (state 3 → A). |
| 6 | Near / at cap | Under block 2 the cap bar. Used ≥ 80% of the cap: amber, "close to the cap · {used} of {cap}%". Used ≥ cap: red, "cap reached · {name}'s agents are stopped until the week resets". Same tokens as the Week planner status (amber/red). |
| 7 | Pending removal | Fields disabled and struck, notice (flow C), `Keep in the plan`, table row of the project replaced by an "other sessions +" hint. Save enabled, labelled `Save` (not "Remove"). |
| 8 | Validation error | Inline under the field, error token, field border red: weight "Weight must be 0 or more" (also non-number), cap "Hard cap must be between 0 and 100". Save disabled while any is shown. Nothing is saved. |
| 9 | Load / save error | Box (same as the sync error box) above the foot: load → "Couldn't load {name}'s allocation: {error}" + `Retry` `r`; save → "Couldn't save: {error}. Nothing was changed." The form keeps the edits. Core messages are shown verbatim, e.g. "set up the weekly plan first". |
| 10 | Plan editor row | Week planner › Edit plan table with 2 normal rows, then one "not in plan" row muted (name + tag), then "other sessions". Plus the same row once a weight is typed (normal). |

Draw 1–9 as one **states** board like `04-project-linear-states.png`, 4 and 5
may share a frame with the changes marked; 10 is a crop of the plan editor.

## Copy (exact)

- Lead: `{name}'s share of the week. Agents of {name} are charged to it; the rest of the plan is in the Week planner.`
- Share: `{share}% of the plan` (live; the table's Share column agrees).
- Used: `used {x} pp this week` (pp = percentage points of the week; one decimal).
- Table title: `Rest of the plan`. Column "Allocation", last row `other sessions`.
- Add button: `Add to the weekly plan`. Remove: `Remove from the plan`. Undo remove: `Keep in the plan`.
- No plan button: `Set up the weekly plan`. Link: `Edit the plan in the Week planner`.
- Toast on save: `Saved {name}'s allocation · this week's usage kept`. Removed: `Removed {name} from the plan · this week's usage kept`.
- Plan editor tag: `not in plan`. Info line addition: `Projects not in the plan are charged to “other sessions”; give one a weight to add it.`
- Hard cap hint: `stops {name}'s agents for the week at this % of the week`.

## Keyboard map

Same model as Task backend: `⌘+⏎` works anywhere in the pane; single keys only
act when no `input`, `textarea` or `select` has focus and no modifier is held.

| Key | Action | When |
|---|---|---|
| `⌘+⏎` | Save | State 5, 7 (not 8) |
| `esc` | Cancel: discard edits, back to saved values; in 7 = Keep in the plan; in a field it first blurs it | dirty |
| `a` | Add to the weekly plan | state 3 |
| `x` | Remove from the plan | state 4, 5 |
| `g` | Open Week planner › Edit plan | states 2–4 (not with unsaved edits: Save or Cancel first) |
| `r` | Retry | state 9 load error |
| `⏎` / `space` on a focused button | press it (single keys skip when a button is focused, as Task backend) | |

Keys line under the foot shows only the keys valid in the current state, e.g.
state 4: `x` remove · `g` week planner.

## Decisions that refine or change the earlier brief

1. **`g` opens Edit plan directly** (not "`g` then `e`"); single key, no chord.
2. **`x` added** for Remove; `a` only in state 3 (it can't clash: Add and Remove are never shown together).
3. **The project's own row sits in the "Rest of the plan" table**, highlighted and live, instead of a separate share sentence only; the section title stays "Rest of the plan".
4. **Weight 0 = pending removal** (state 7), mirroring the plan editor.
5. **Cap thresholds**: amber from 80% of the cap, red at the cap (the brief left them open).
6. **Cancel/Save disabled when clean**; Save also disabled on a validation error.
7. **Removal toast** has its own text (the save toast would say "Saved" for a removal); no modal anywhere.

## Constraints

- Tokens only (`ui/src/styles/tokens.css`); dark, light and high contrast.
- Works at 1280 and 1512; the table scrolls horizontally never, it truncates the name.
- Reuse the plan editor's tier select and number fields; no new component beyond the section.
- Out of scope (spec): allocations per feature or team, editing other projects' rows, calendar / reserve.

## Hand-off to ui

Frames in `docs/design/Orchestrator.figcraft`: **04 Project settings ·
Allocation · 1512** (state 4), the **states** board (1–9), and the plan editor
row variant (10), exported as PNG to
`.orchestrator/features/add-allocation-in-project-settings/design/`.
