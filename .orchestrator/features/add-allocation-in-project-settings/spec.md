# Add Allocation In Project's Settings: spec

## Problem

FR-109 gives each project a **budget share**: its allocation in the weekly plan
(FR-802: weight, tier, optional hard cap, FR-811). Today:

1. **Project settings › Allocation is empty.** `ProjectSettingsPane.svelte`
   falls through to "Not designed yet".
2. **A project added after the first plan can't get an allocation.**
   `PlanForm::from_plan` (`crates/orch-app/src/setup.rs`) only proposes one row
   per project while the plan holds nothing but `default`. Once a plan is saved,
   the form lists the plan's allocations only, so a newer project has no row in
   Week planner › Edit plan. Its agents are charged to "other sessions", and the
   user can't change that.
3. **Agents are charged to an allocation inconsistently.** Solo sessions
   (`core.rs` `start_session`) use the project **id**, and only when the plan
   already has it (otherwise `default`, fixed for the agent's life). Team agents
   (`agent-runtime/src/team.rs`) use the project **name**, whether or not the
   plan has it. Plan allocation ids are project ids.

## Goal

From a project's settings, the user sees how much of the week that project gets
and changes it: add the project to the plan, set its weight, tier and hard cap,
or take it out. The Week planner's plan editor lists every project too, so any
project can be added there as well.

## Scope

### 1. Project settings › Allocation (new section)

The section shows the project's allocation in the **current weekly plan**:

- **Not in the plan**: says the project's agents are charged to "other sessions",
  and offers **Add to the weekly plan**. Adding pre-fills weight = the mean
  weight of the plan's project allocations (rounded, at least 1; 1 when there
  are none), tier normal, no hard cap. Nothing is saved until Save.
- **In the plan**: weight, tier (critical / high / normal / low), hard cap (%,
  empty = none), and its **share** (weight / sum of all weights, live as the
  weight is typed). Plus **used so far this week** (pp of the week) and, with a
  cap, how close it is to it.
- **Context**: the other allocations of the plan, read only (name, weight,
  share, tier), "other sessions" last, so the user sees what the share is
  relative to. A link opens Week planner › Edit plan to change them.
- **Remove from the plan**: the project goes back to "other sessions" (same as
  weight 0 in the plan editor). Applies at Save.
- **No plan confirmed yet** (`plan_confirmed` false): the section shows the
  even split the core would propose, read only, and a button to set up the plan
  in the Week planner. It doesn't confirm a plan by itself.
- Save `⌘+⏎`, Cancel `esc`, like the other sections. Save toast:
  "Saved {name}'s allocation · this week's usage kept". Errors inline.

Saving replaces only this project's allocation in the current plan; the
calendar, reserve and the other allocations are untouched. The Governor replays
the week's usage into the new plan (`Runtime::replan`), as Edit plan does.

### 2. Week planner › Edit plan lists every project

`PlanForm::from_plan` appends each registered project that has no allocation as
a row with weight 0, before "other sessions". `PlanEditor.svelte` shows them
muted, "not in plan"; typing a weight above 0 adds the project at Save (0 rows
are already dropped by `PlanForm::apply`). The first-plan even split is
unchanged.

### 3. One rule for charging agents

- An agent's allocation is its feature's allocation if set, else its **project
  id** (team agents: id, not name). It is recorded as-is, whether or not the
  plan has it.
- The Governor resolves an allocation the plan doesn't have to `default`
  ("other sessions") wherever it charges, schedules or caps. So adding or
  removing a project applies to its running agents too, from the save on, and
  after the replay the project's usage earlier this week counts toward its new
  allocation (usage rows and agent records that hold `default` for a solo
  session started before the project had an allocation stay in "other
  sessions"). Every reader resolves the same way, including the planner's
  live-agent count per allocation (`core.rs`).
- Existing records holding a project **name** resolve to that project's id when
  no allocation has that name (no migration).

## Out of scope

- Allocations per feature or team (the field exists; no UI here).
- Several projects sharing one allocation.
- Renaming an allocation when its project is renamed; cleaning up allocations of
  removed projects.
- Changing the calendar, reserve or other projects' rows from project settings.

## Acceptance

1. A project registered after the plan was saved: Project settings › Allocation
   says it is charged to "other sessions"; Add → weight/tier/cap → Save puts it
   in the plan. The Week planner lists it with that weight and tier, and its
   agents (running and new) are charged to it.
2. Changing the weight updates the share live; Save keeps the week's usage and
   the other allocations as they were.
3. Remove → Save: the project is gone from the plan, its agents go back to
   "other sessions".
4. Week planner › Edit plan shows projects not in the plan at weight 0; setting
   a weight and saving adds them.
5. No confirmed plan: the section shows the proposed split read only and links
   to the Week planner.
6. Invalid cap (outside 0–100) or negative weight: inline error, nothing saved.
7. A team agent and a solo session of the same project land in the same
   allocation.
8. `make test` and `make lint` pass; new Rust tests cover the core commands,
   `from_plan` with missing projects, and the allocation resolution; UI tests
   cover the section's states.

## Team

- **ux**: flows and states of the section and the plan editor change
  (`ux-brief.md`).
- **ui**: FigCraft frames in [`Orchestrator.figcraft`](https://drive.google.com/open?id=1n4O57ntQDFvv1-pw3flJkmcHh4MeTLVs&usp=drive_fs) (04 Project
  settings · Allocation + states, plan editor row), exported to `design/`.
- **back**: the two commands (`api-contract.md`), `from_plan`, the charging rule.
- **front**: the section in `ProjectSettingsPane.svelte` (or a component it
  uses), the plan editor rows, tests.
