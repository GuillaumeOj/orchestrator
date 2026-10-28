# governor

The usage-pacing Governor of Orchestrator ([ADR-007](../../docs/adr/007-usage-pacing-governor.md),
SPEC §4.8 FR-801–FR-820, J8, J9, NFR-9). It spreads agent work over the plan week so the weekly
limit lasts until the reset, keeps the 5-hour window clear for interactive work, and logs every
decision with its reason.

It is a **pure library**: no I/O, no async, no clock. Every method takes `now`. The Rust core feeds
it statusline samples and hook events, and carries out its decisions. The same inputs always give
the same decisions, which is what makes the [simulator](../pacing-sim) usable as an acceptance test.

## API in one screen

```rust
use governor::jiff::{civil::time, Timestamp};
use governor::*;

let week_start: Timestamp = "2026-10-19T07:00:00Z".parse()?; // Monday 09:00 Paris, the weekly reset
let plan = WeeklyPlan {
    id: "2026-W43".into(),
    week_start,
    reset_at: "2026-10-26T08:00:00Z".parse()?,
    calendar: WorkingCalendar::weekdays("Europe/Paris", time(9, 0, 0, 0), time(19, 0, 0, 0)),
    reserve_pct: 10.0,
    off_hours_allowance_pct: 0.0,
    shadow_mode: false,
    allocations: vec![BudgetAllocation {
        id: "A".into(), target: "project A".into(), weight: 50.0, tier: Tier::High, hard_cap_pct: None,
    }],
};
let mut gov = Governor::new(plan, PacingPolicy::default(), week_start)?;
let now = week_start;

// Admission point 1: an app-delivered turn (or a self-started one, FR-410).
let d = gov.request_turn(TurnRequest::task("A-1", "A", Tier::Normal), now);
assert_eq!(d.effective, Verdict::Admit); // or Queue { reason, estimated_start, .. }

// Statusline + cost, per agent, with the API response time (FR-801).
gov.ingest(&UsageSample {
    ts: now,
    agent: Some("A-1".into()),
    cost_delta: 0.12,
    five_hour: Some(WindowReading { used_pct: 3.0, resets_at: "2026-10-19T12:00:00Z".parse()? }),
    weekly: Some(WindowReading { used_pct: 1.0, resets_at: "2026-10-26T08:00:00Z".parse()? }),
    provenance: Provenance::Observed,
    model: None,
});

// Admission point 2: PreToolUse. A Deny ends the turn ("paused by governor").
let d = gov.tool_call(&ToolCallRequest { agent: "A-1".into(), interactive: false }, now);
assert!(d.effective.is_admit());

// The control loop: call every minute or so. Returns queued turns to deliver now.
for release in gov.tick(now) {
    println!("deliver {:?} to {}", release.kind, release.agent);
}
let released = gov.turn_ended(&"A-1".into(), now); // Stop / StopFailure

let snap = gov.snapshot(now); // W, G, plan, e, projected, eta, state, caps, 5h, estimator
println!("W {:.0} % · plan {:.0} % ({:+.0}) · {}", snap.used, snap.plan, snap.error, snap.state);
for action in gov.actions() {
    println!("{} {}: {} → {}", action.ts, action.kind_name(), action.reason, action.expected_effect);
}
# let _ = released;
# Ok::<(), Box<dyn std::error::Error>>(())
```

| Call | When the core calls it |
|---|---|
| `request_turn(TurnRequest, now) -> Decision` | Before delivering a turn; also for a self-started turn (`self_started: true`) or a user turn (`interactive: true`, always admitted). |
| `tool_call(ToolCallRequest, now) -> Decision` | From `orch-hook` on PreToolUse. `Deny { reason }` → answer deny; the turn ends and a `Continue` entry waits in the queue. |
| `turn_ended(agent, now) -> Vec<Release>` | Stop, StopFailure, interrupt. Frees the slot. |
| `ingest(&UsageSample)` | Every statusline sample (with cost delta). `None` windows stay unknown. |
| `rate_limit_hit(agent, window, resets_at, now) -> LimitResponse` | StopFailure `rate_limit` (J9). `cancel_auto_continue` = send Esc. |
| `tick(now) -> Vec<Release>` | Periodically. Limits, burn rates, 5h guard, state, ask timeouts, alerts, releases. |
| `pause_all`, `resume_all`, `pause_agent(agent)`, `resume_agent(agent)` | Pause all, or one agent: its autonomous turn stops at the next tool call, its queued work waits (FR-805, FR-811). |
| `run_now(entry)`, `cancel(entry, now)` | Run a queued entry past every cap (the memory cap included), or drop it; both logged (FR-809). |
| `set_process_slots(live, cap, now) -> Vec<Release>` | The host's live `claude` processes and the memory-pressure cap on them (FR-203). Spawns (`TurnRequest.needs_spawn`) wait for a slot. |
| `start_sprint(extra_pp, now)` | Sprint mode until the end of today's working time (FR-811). |
| `answer_ask(id, accept)`, `undo(id)` | "Ask" actions (FR-810) and undo from the log (FR-809). |
| `new_week(plan, now)` | After the weekly reset. Calibration and the queue carry over; budgets don't. A plan whose week contains the current week's start (a realignment, FR-802) keeps the week's usage. |
| `cost_class(&sample)`, `replay(&sample, class, allocation)` | Store the class with each sample; after a restart, replay the week's samples so the accounting (interactive vs autonomous, per allocation) and the calibration come back. |
| `hold_spawn(agent, allocation, tier, live, cap, now)` | The memory cap checked again at spawn time: an admitted spawn with no slot goes back to the queue. |
| `carry() -> Carry`, `restore(carry, keep, now)` | What a restart or a replan would otherwise lose: pauses, deferrals, the pending ask, sprints, limits, the running turns and the queue (ADR-007 "State across restarts"). |
| `snapshot`, `queue`, `actions`, `next_action_preview`, `paused_agents`, `stats` | Read-only views for the UI and the weekly report. |

`Decision.effective` is what the host must do; `Decision.proposed` is what the policy wanted. They
only differ in **shadow mode** (FR-812), where everything is admitted and the would-be decisions are
logged as `ActionMode::Shadow`.

Types mirror SPEC §7: `WeeklyPlan`, `BudgetAllocation`, `PacingPolicy` (with `Thresholds` and
`LiveCaps`), `QueueEntry`, `GovernorAction`, `Calibration`, `ForecastSnapshot`, `UsageSample`.
All are `serde` types.

## The control law as implemented

**Measurements.**
- `W` (weekly) and `F` (5 h): the most recent sample per window, across all agents, by sample time.
  A window missing from a response is unknown. A whole-percent reading `n` below 100 is read as
  `n + 0.5`.
- Between observations, `W = W_obs + cost_since / k + u · Δt`. An observation older than 15 min is
  *estimated*. The margin is `min(margin(confidence), extrapolated delta)`, where
  `margin(c) = 15 − 5c` pp. The full margin applies only before the week's first reading when the
  Governor did not see the reset.
- The weekly reset is a reading of 0: the week's first reading calibrates against it, whether the
  app ran from the reset, started later, or replays the week's samples after a restart. A late
  start and a restart therefore give the same `k`, `u` and forecast.
- `k` (cost per 1 % of the week) and `u` (unattributed pp per wall hour) come from an
  exponentially weighted least-squares fit of `ΔW ≈ ΔA/k + u·Δt`. It runs over calibration intervals
  of at least 3 pp and 30 min between observations, with weak priors `k = k0`, `u = 0`.
  Confidence is `n_eff / (n_eff + 3)`.
- `U_so_far = W_obs − A_week/k`, and `U_forecast = U_so_far + u' × wall hours left`, where
  `u' = (u·h + u0·H0) / (h + H0)` shrinks the fitted rate toward a prior `u0`
  (`unattributed_prior_pct_per_h`, default 0) by the hours `h` of the week observed so far, with
  `H0 = unattributed_prior_hours` (default 48 h). One heavy day of outside use is then not
  extrapolated over the whole week. All of the week's estimates use `u'`.

**Curve.**
- `H(t)` counts weighted working hours from the calendar: blocks per day in the plan's time zone,
  DST-safe, with holidays. Off-hours count only if `off_hours_allowance_pct > 0`; they then hold
  exactly that share of the week.
- `ideal(t) = H(t)/H_week × (100 − reserve − U_forecast)`.
- The error is on the governed share (ADR-007, decision 2026-10-21): `G = W − I_used − U_so_far`,
  `e = G − ideal(t)`, where `I_used` is interactive usage (critical agent work stays in `G`).
  `plan(t) = ideal(t) + I_used + U_so_far`, so `e = W − plan(t)` too.
  `reserve_left = 100 − W − autonomous plan left − unattributed left`.

**Projected total.**
- `P = W + burn × H_left + u × off-hours left`: the total W projected at the reset. Off-hours here
  are the wall hours with weight 0, where outside usage goes on and the burn is not measured.
- `burn` is an EWMA per weighted hour with τ = 24 weighted hours. Per weighted hour, so the
  projection follows the curve: half-weight blocks and sprints. It is bias-corrected: the weight
  the measurement does not have yet (after the reset or a restart) goes to the on-plan pace,
  `((budget − ideal) + u' × working wall hours left) / H_left`. Outside usage that already shrank
  the agents' budget therefore does not by itself project the week over the limit.
- `eta` (shown only) is when the total reaches 100 % along the curve at that burn.

**States.**
- Green, Amber, Red and Catch-up follow ADR-007's thresholds, with hysteresis and a minimum dwell of
  30 min on every transition. Red enters on `e > +8` or `P > 98 %` and exits on `e < +5` and `P < 96 %`.
- Entering Red proposes deferring a low-tier allocation (see "Asks" below).
- Entering Catch-up lifts the deferrals.

**Live caps.**
- `LiveCaps::from_base(3)` gives Green 3, Amber 2, Red 1, Catch-up 5, and 0 off-hours.
- In a block of weight `w < 1` the cap is `ceil(cap × w)`.
- The 5h guard caps it to 1.

**5h guard.**
- Projected `F` at the 5h reset (`F + EWMA rate × hours to reset`, τ = 2 h) above 85 % caps live
  agents to 1. The cap is lifted below 80 %.
- `F` itself above 85 % admits only interactive and critical work, and holds autonomous turns at
  their next tool call.

**Admission order.**
1. Interactive work is always admitted.
2. Pause all, a paused agent and an active plan limit queue the work.
3. A spawn (a suspended agent's turn) waits for a process slot under memory pressure.
4. Critical work is admitted.
5. Then, in order: `F` above 85 % (5h guard), a deferred allocation, a hard cap, and in Red
   anything below `red_min_tier` (Normal) are queued.
6. A live cap reached, or eligible work already waiting ahead, queues the turn.
7. Otherwise the turn is admitted.

At each tool call of a running autonomous turn, Pause all, a paused agent, a hit limit and `F`
above 85 % hold it, and so do a deferral of its allocation and a hard cap it crossed during the
turn. Red's tier rule and the live cap apply at turn start only.

**Queue.**
- Strict by effective tier: Critical first, then High, then Normal, then Low.
- Aging gives +1 tier per 2 h waited, up to High.
- Within a tier, weighted fair queuing on `usage / weight` (live turns included). An allocation
  that becomes backlogged starts at the others' level (start-time fair queuing): unused weight is
  redistributed and idle time is not banked.
- Each entry carries its reason, an estimated start, and the cost of "Run now": in pp of the week,
  and in % of the 5h window once a 5h reading was seen. `binding` names the window that holds it
  (the 5h window under the 5h guard or a 5h limit), so the UI shows that cost first. The 5h cost
  per pp is fitted from two readings of the same 5h window (cost between them / ΔF; outside usage
  makes it err high), else `k / five_hour_per_weekly_pct` (default 5).
- The next-action preview names the entry the next release would pick: eligible and not live.

**Asks** carry a default and a 20-min timeout. The default is `Apply`; `Skip` is configurable.
Deferrals, sprints, Pause all and an agent's pause can be undone. Undoing a pause logs the resume;
Resume all (or an agent's resume) marks the pauses it ends as undone.

**Memory slots (FR-203).** The host reports its live processes and the cap that memory pressure
leaves (`set_process_slots`). A turn that must spawn a process waits as `MemoryPressure { cap }`
with no estimated start, and is released when a slot frees. This is a memory limit, not pacing:
it is **enforced in shadow mode too**, where the queue holds only these waits. Run now overrides it
(FR-203: the user may exceed the cap).

**Carry across restarts.** `carry()` holds the pauses (always restored), the limits until their
reset, and, for the same plan week only, deferrals, the pending ask (with its deadline) and sprints
(re-applied to the curve with their boost). The queue comes back with its ids and `enqueued_at`
(aging goes on); `keep` lets the host drop entries whose agents are gone. `live` holds the running
turns: a replan restores them first, so the caps still count them; after a restart the host
empties it (the processes are gone). Restored overrides are logged again as actions ("restored
after a restart"), so they can be undone.

**Spawn-time check.** `hold_spawn(agent, allocation, tier, live, cap, now)`: the host found no
process slot when it was about to spawn an admitted turn. The turn leaves the live set (no turn
statistics) and waits as a memory wait, also in shadow mode.

**J9.**
- A 429 marks the window limited until `resets_at`. The agent's work is queued as `Continue`.
- `cancel_auto_continue` is true by default (`AutoContinue::CancelAndReschedule`).
- Other agents stop at their next tool call. At the reset, the queue drains in priority order.

**Watchdog.** A turn over its `TurnCeiling` (pp or minutes) is denied at its next tool call. It is
logged for the Inbox and not re-queued.

**FR-410.** A self-started turn goes through `request_turn`. If it is not admitted, its tool calls
are denied and a `Continue` entry holds its place. Its cost always counts against the agent's
allocation.

## Simulator

`crates/pacing-sim` replays a week minute by minute, with a hidden truth the Governor never sees:
- true usage, integer statusline readings on each API response, and attributed cost;
- demand per allocation, worked through by agents sized for their backlog;
- the user's interactive bursts and usage outside the app;
- optionally a forced 429 and a sprint.

```bash
cargo run --release -p pacing-sim -- --scenario nfr9-130          # 130 % demand (NFR-9)
cargo run --release -p pacing-sim -- --scenario nfr9-60           # 60 % demand (NFR-9)
cargo run --release -p pacing-sim -- --scenario full --seed 3     # + forced 5h 429, sprint, more interactive
cargo run --release -p pacing-sim -- --scenario full --shadow     # same week, decisions logged only
cargo run --release -p pacing-sim -- --demand 100 --seed 9 --json # any demand, JSON summary
cargo run --release -p pacing-sim -- --dump-scenario week.json    # write the week as JSON…
cargo run --release -p pacing-sim -- --file week.json             # …edit it, or record one, and replay it
```

Each run prints a summary and writes two files to `out/` (`--out DIR` to change):
- **Summary**: final weekly %, time of any limit hit, queue waits by tier, actions by kind, time
  per state, work done per allocation, and estimator accuracy.
- `out/<scenario>.csv`: one row every 5 minutes with `true_weekly`, `observed_weekly`, `used`,
  `plan`, `ideal`, `error`, `state`, `live_cap`, `live`, `queued`, the 5h values, `burn`, `k`,
  `confidence`, `unattributed_so_far`, `reserve_left` and `interactive_used`.
- `out/<scenario>.actions.jsonl`: the full `GovernorAction` log.

To plot the CSV:

```bash
python3 -c "import pandas as p; d=p.read_csv('out/nfr9-130.csv'); d.plot(x='local', y=['true_weekly','plan','ideal']).figure.savefig('out/curves.png')"
```

A recorded week uses the same JSON structure (`Scenario`): tasks with arrival times and sizes,
interactive and outside bursts, and the plan and policy.

### Results (default policy, 200 seeds per scenario)

| Scenario | Weekly % at reset | Weekly limit hit | Queued |
|---|---|---|---|
| `nfr9-130` (130 % demand) | 89.6–97.4 (mean 93.2), max 95.6 at end of working time | never | always (paced) |
| `nfr9-60` (60 % demand) | 53.3–60.3 (all its demand; lower values are tasks that arrived too late to finish) | never | **never** |
| `full` (115 %, 7 % interactive, 6 % outside, forced 5h 429, sprint) | 86.2–97.9 (mean 91.8) | never (the forced 5h 429 is handled) | always |

`cargo test` runs the NFR-9 acceptance tests on seeds 1–20 (`crates/pacing-sim/tests/nfr9.rs`):
- 130 %: no weekly limit hit, and the week ends at 85–100 %;
- 60 %: zero queued turns and zero denied tool calls;
- no oscillation: every state lasts at least the dwell;
- no starvation: with deferral declined, every queued turn starts within 5 working hours;
- the J9 flow, shadow mode, sprint, determinism, and a JSON round trip of a recorded week;
- a hard cap (FR-811) holding its allocation within 1.5 pp of the cap in 130 % weeks.

The simulator's assumptions drive these numbers. Each agent burns 0.8 % of the week per hour
(±25 %). One weekly percent costs 5 % of the 5h window (`five_hour_per_weekly_pct`). The reserve is
10 %, with 5 % interactive work, 2 % critical work and 4 % outside usage in the NFR-9 weeks.

With a tighter 5h window (6 % per weekly percent), the 5h guard queued some turns in 9 of 100 of
the 60 % weeks; the 130 % results were unchanged. The real ratio should be measured (see the
proposed edits).

### Known limits seen in simulation

- With the governed-share error, interactive and outside usage no longer slow the agents, so at
  130 % demand heavy weeks end at about 93–99 % and the projection rule does the braking late in the
  week. A front-loaded interactive burst raises the burn and can enter Red early in the week, since
  the burn is extrapolated over all the hours left.
- Entering Red asks to defer the low-tier allocation. The ask defaults to apply, so in 85 % of those
  weeks the low-tier project gets about 20 % of its demand done. Declined, aging still runs it.
- Small outside usage (~4 %/week) is barely identifiable from whole-percent readings: `u` is often
  estimated near 0, and `k` absorbs it.
- Amber (2 agents) is roughly neutral when interactive and critical work run at about 0.3 pp/h. The
  error then sits between +1 and +8 until Red corrects it, so heavy weeks end at about 90 + e.

## Readings of the spec, and proposed spec edits

Where ADR-007 and the SPEC were ambiguous or contradicted each other, this crate takes the simplest
reading that passes NFR-9. Each one is listed here as a proposed edit. SPEC and ADRs are unchanged.

1. **Unattributed usage is part of the plan, not only subtracted (FR-803/813).** Comparing `W` with
   `ideal(t)` alone counts outside usage twice: once inside `W`, once in the budget's
   `− U_forecast`. *Edit*: `e = W − plan(t)` with `plan(t) = ideal(t) + U_so_far` (now `+ I_used` as well, item 2). This is the
   "plan" shown in J8's title bar.
2. **The reserve is a credit for interactive work (FR-804, decision 2026-10-21).** This crate first
   counted interactive usage in the error; ADR-007 now takes it out (`G`), and the projection rule
   on the total prevents the lock-out instead. Critical agent work stays in `G`: taking it out too
   made 3 of 20 130 % weeks reach 100 %.
3. **`k` and `U` are fitted together (FR-814, ADR measurements).** Two separate EWMAs are circular:
   `k` fitted from the observed deltas absorbs `U`. *Edit*: "one exponentially weighted fit of
   `ΔW ≈ ΔA/k + u·Δt`", which is the EWMA of the fit's statistics.
4. **The estimator margin is bounded by what is extrapolated.** A flat 10–15 pp whenever the value is
   stale jolted the state at every idle gap: each night, and each morning before the first response.
   It also made 60 % weeks queue. *Edit*: the margin is `min(10–15 pp by confidence, estimated
   delta)`, and the full margin applies only before the week's first reading when the Governor did
   not see the reset. The weekly reset counts as a reading of 0 (applied in SPEC v0.9).
5. **Whole-percent readings.** This crate reads `n` as `n + 0.5`, assuming the statusline rounds
   down. That removed the last weekly-limit hits at 130 %. *Edit*: add to R10 a check of whether
   `used_percentage` is floored or rounded.
6. **The exhaustion rule is a projected total in pp (ADR-007, decision 2026-10-26).**
   - The earlier `eta < reset − 4 h` / `eta ≥ reset` pair lost its hysteresis late in the week: with
     the reset days after the last working hour both mean "runs out before working time ends", and
     with the governed-share error it flipped Red/Green every 30 min (up to 62 changes a week).
   - The burn is per weighted hour (EWMA τ = 24 h), projected over the weighted hours left. Per
     active hour, a sprint's higher burn was extrapolated over the whole week and the sprint ran
     nothing extra. With a 6–12 h τ, busy mornings sent 60 % weeks into Red.
7. **Red admits tier ≥ normal (ADR table vs ADR pseudo-code).** The table says "only tier ≥
   normal", while the pseudo-code queues `tier <= normal` in Red. This crate follows the table and
   J8 ("queues low-priority team tasks"), as the configurable `red_min_tier = Normal`. *Edit*: fix
   the pseudo-code.
8. **No autonomous work off-hours without an allowance (new).** ADR-007 is silent. Without this,
   agents keep running at night at Red's cap of 1, and that alone exhausts the week. *Edit*:
   `LiveCaps.off_hours = 0` by default; FR-816's off-hours scheduling raises it.
9. **The live cap follows the block weight (new).** A block of weight 0.5 halves the plan's pace,
   but the cap stayed the same. Friday-afternoon half-weight hours let the error grow at the very
   end: max 100.6 % before, 97.4 % after. *Edit*: `cap = ceil(cap × w)` for `w < 1`.
10. **The dwell applies to every transition, escalation to Red included.** ADR-007's "minimum dwell
    30 min" is unqualified, and the 5h guard covers fast spikes.
11. **The 5h guard has hysteresis and a defined projection.** The ADR gives neither. *Edit*: enter
    above 85 % projected, exit below 80 %, with the projection
    `F + EWMA rate (τ = 2 h) × time to the 5h reset`.
12. **Pause all and a hit limit also hold critical work.** In the ADR, critical work is admitted
    before any other check. Interactive work is still never blocked. *Edit*: order the admission
    rule as interactive → pause/limit → critical → the rest.
13. **Aging stops at High.** Only work that is critical from the start gets strict priority.
14. **The deferral ask's target and lifetime.** On entering Red, the ask targets the allocation
    below `red_min_tier` with the most work. It lasts until the reset, is lifted by Catch-up, and can
    be undone. Its default of "apply" starves that allocation in most 130 % weeks. *Edit*: consider
    defaulting to skip, or deferring only what is queued at that moment.
15. **NFR-9 "never queues".** This is checked literally: zero queue entries. It requires a 60 %
    week whose concurrency fits the base cap (3 agents) and whose work is offered in working time.
    *Edit*: state that the base cap is a memory cap too (NFR-1b), so a 60 % week with more parallel
    work queues by design. Also state the assumed 5h-per-weekly ratio.
16. **Undo (FR-809, "every governor action … can be undone").** Undo applies to deferral, sprint,
    Pause all and an agent's pause. State changes, caps and holds are re-decided by the loop, and undoing them has no
    meaning. *Edit*: "every user-affecting action".
17. **A self-started turn that is not admitted** keeps running until its first tool call, which is
    denied; its place in the queue is a `Continue` entry. This matches FR-410 and states how.
18. **The memory cap counts processes and is enforced in shadow mode (FR-203, FR-812).** The
    Governor's live caps count admitted autonomous turns; memory pressure caps live `claude`
    processes, so it holds only spawns (resuming a suspended agent). A memory limit is not pacing,
    so shadow mode does not lift it. *Edit*: say so in FR-203 and FR-812.
19. **Deferral and hard caps hold running turns (FR-811).** A turn that crosses its allocation's
    hard cap, or whose allocation is deferred, stops at its next tool call, like Pause all.
    *Edit*: "a running turn stops at its next tool boundary" in FR-811.

## Not done yet

- FR-815 per-model weekly windows (S priority): `Window` has room for them.
- Resume-cost awareness (`prompt_cache.expires_at`, R6): the queue does not prefer warm sessions yet.
- 5h peak smoothing (deferred by ADR-007). Per-model estimator weights are in (FR-814, off by
  default: `PacingPolicy::per_model_weights`).
- Pausing a team at its turn boundary (FR-805): the runtime pauses every agent of the team run
  (`pause_agent`; agent-runtime "Teams"); per-agent pause and Pause all
  cover P1b.
- "Start now" vs "Run now" is the host's call (the Governor does not know whether an agent ever
  ran).
