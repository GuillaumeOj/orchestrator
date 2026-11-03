# Add Setting For Live Cap: spec

## Problem

The number of agents that may run at once is fixed at **3** in the app
(`RuntimeConfig::base_cap`, `crates/agent-runtime/src/runtime.rs`). Only the
headless `orchd serve --cap N` can change it; the desktop app always starts
with 3, and nothing in Settings edits it (Settings › Pacing policy is a "Not
designed yet" placeholder).

That one number drives two caps:

| cap | where | from base `b` (3 today) |
|---|---|---|
| **Governor live cap**: admitted autonomous turns at once, per pressure state | `LiveCaps::from_base` (`crates/governor/src/plan.rs`), set in `restore_governor` | Green `b` (3) · Amber `ceil(b/2)` (2) · Red 1 · Catch-up `b + 2` (5) · scaled by `ceil(cap × w)` in a block of weight `w < 1` · 1 under the 5h guard |
| **Process cap**: live `claude` processes (FR-203), Leads and the Overwatch not counted | `resources::effective_cap` (`crates/agent-runtime/src/resources.rs`) | `b` · `b − 1` on memory *warn* (at least 1) · 1 on *critical* |

So in practice the user sees 2 live agents (Amber, a half-weight hour, or
memory *warn*) and at most 3. The user wants to raise it and tune it from
Settings.

## Goal

The user sets the base live cap in **Settings › Pacing policy**. It applies
at once, survives a restart, and every cap derived from it follows.

## Scope

In:

1. **Live agents** (the base cap `b`): 1–20, default 3. Drives both caps in
   the table above with the existing formulas. One number, not one per state:
   the per-state ratios (Amber half, Red 1, Catch-up +2) are what keeps the
   week on plan, and the formulas already scale them.
2. **Off-hours live agents** (`PacingPolicy::off_hours_live_cap`, FR-816):
   0–20, default 1. Already stored and validated by the core (`PlanForm`),
   but no UI edits it. Same section, same save.
3. The **Settings › Pacing policy** section replaces its placeholder with
   these two fields, the caps they give per state (read only, updated as the
   user types) and the cap in effect now.
4. Design: UX copy/states/keys, then the UI frame on FigCraft page
   `04.7 Settings`, exports in `design/` of this feature.

Out:

- Per-state overrides (Green/Amber/Red/Catch-up typed one by one).
- The other pacing values (thresholds, reserve, tiers…): the Week planner
  edits the plan; the rest of the section stays out of this feature.
- A separate process cap: the process cap stays derived from the same base
  (SPEC NFR-1b: the base cap is a memory cap too).

## Behaviour

- **Stored** as the app setting `governor.live_cap` (a number), like
  `governor.enforce` and `permissions.hold_secs`. No setting = 3. A stored
  value outside 1–20 is ignored (default used, warning logged).
- **`orchd serve --cap N`** still wins for that run (not stored). Without the
  flag, `orchd` reads the setting like the app.
- **Applies now**, no restart: the runtime's base cap is updated, the Governor
  is rebuilt with the new `LiveCaps` (the same path as `replan`: the week's
  accounting, queue, pauses and sprints carry over), and the process slots are
  re-synced, so work queued for a slot starts if the cap went up. The usage
  snapshot the UI shows (`live_cap`) refreshes.
- **Lowering it never stops a working agent.** Running turns finish; new
  turns wait in the queue (`QueueReason::LiveCap`). Idle agents above the
  process cap are suspended by housekeeping as today (least recently active
  first, Leads and the Overwatch exempt); when memory pressure is *normal*
  the suspend reason is `live_cap`, not `memory_pressure`, so the weekly
  report doesn't count them as pressure suspends.
- **Shadow mode**: the Governor's cap is logged, not enforced (as today); the
  process cap applies anyway (FR-812). The section says so while shadow mode
  is on.
- **Audit**: a change logs an event `live_cap_set` `{from, to, by: "user"}`
  (and `off_hours_live_cap_set` likewise).
- Validation errors are returned as text for the field: `live agents 0: use
  1–20`, `off-hours live agents 25: use 0–20`.
- **Higher cap, same week**: raising the cap doesn't raise the weekly budget.
  The Governor still paces the week (Amber/Red still lower the cap); more live
  agents only use the plan sooner. Above 6 the section shows a soft note about
  memory (≈ 190–255 MB per agent, FR-203). No hard block.

## Docs to update (backend task)

- `docs/SPEC.md` FR-203: "default 3, set in Settings › Pacing policy (1–20)".
- `crates/governor/README.md` "Live caps" and `crates/agent-runtime` docs:
  the base comes from the `governor.live_cap` setting.

## Acceptance

1. Settings › Pacing policy shows Live agents (3) and Off-hours live agents
   (1) on a fresh install, with Green 3 · Amber 2 · Red 1 · Catch-up 5 and
   memory 3 / 2 / 1.
2. Setting 6 and applying: the pacing indicator shows `live x/6` in Green
   without a restart; a queued turn waiting for a slot starts; after an app
   restart the value is still 6.
3. Setting 1 with 3 agents working: none is stopped; their next turns queue
   with `QueueReason::LiveCap { cap: 1 }` (shown as the existing "{cap} live agents allowed in {state}" text).
4. 0 or 21 is refused with the field error; nothing changes.
5. `cargo test` and the UI tests pass; new tests cover the setting read at
   start, the change applied live (Governor caps + process slots), the
   bounds, the `--cap` override, and the suspend reason.
