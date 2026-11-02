# Add Allocation In Project's Settings: API contract

Tauri commands (`crates/orch-app/src/commands.rs`, registered in `lib.rs`),
called from the UI with `invoke`. Numbers are JSON numbers; percentages are
0–100; `pp` = percentage points of the week.

## `project_allocation({ project }) -> ProjectAllocationView`

```jsonc
{
  "project": "shop",            // project id = allocation id
  "name": "shop-api",
  "plan_confirmed": true,       // false: no plan saved yet (settings PLAN_KEY)
  "allocation": {               // null when the plan has no allocation for it
    "id": "shop",
    "target": "shop-api",
    "weight": 50,
    "tier": "high",             // critical | high | normal | low
    "hard_cap_pct": 20,         // null = no cap
    "used_pct": 7.4             // Governor allocation_usage_pct this week, pp
  },
  "others": [                   // every other allocation of the plan, plan order, "default" last
    { "id": "blog", "target": "blog", "weight": 30, "tier": "normal", "hard_cap_pct": null, "used_pct": 2.1 },
    { "id": "default", "target": "other sessions", "weight": 1, "tier": "normal", "hard_cap_pct": null, "used_pct": 0.4 }
  ],
  "proposed_weight": 40         // weight pre-filled by Add: mean of the project allocations' weights, rounded, ≥ 1; 1 if none
}
```

When `plan_confirmed` is false, `allocation` and `others` are the even split
`PlanForm::from_plan` proposes (read only in the UI).

Errors: unknown project → `"no project {id}"`.

## `project_allocation_set({ project, form }) -> ProjectAllocationView`

```jsonc
// form: set or add
{ "weight": 40, "tier": "normal", "hard_cap_pct": null }
// form: null (or weight 0) = remove the project's allocation
null
```

- Replaces (or adds, or removes) only this project's allocation in the current
  plan; `target` = the project name. Calendar, reserve, other allocations
  unchanged. `default` is always kept (`PlanForm::apply` rule).
- Applied with `Runtime::replan` (week's usage replayed). Returns the new view.
- Errors (nothing saved): `plan_confirmed` false → `"set up the weekly plan
  first"`; weight < 0, cap outside 0–100 → the same messages as `plan_set`.

## Changed: `plan_form() -> PlanForm`

`allocations` now also lists each registered project with no allocation, with
`weight: 0`, `tier: "normal"`, `hard_cap_pct: null`, placed before `default`.
Shape unchanged (`AllocationForm { id, target, weight, tier, hard_cap_pct }`).
`plan_set` unchanged (weight 0 rows are dropped).

## Charging rule (no API change)

Agents record their feature's allocation, else their project id. The Governor
treats an allocation id that the plan doesn't have as `default` (charging,
fair queuing, tiers, hard caps). Existing agent records holding a project name
resolve to that project's id.
