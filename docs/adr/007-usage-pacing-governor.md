# ADR-007 — Usage pacing: a central Scheduler/Governor

- Status: Proposed · Date: 2026-10-17

## Context
Running many agents on a Max plan can exhaust the weekly limit mid-week. Pacing is a core feature (SPEC §4.8): the budget must last the whole plan week according to the user's working schedule and priorities.

## Decision
1. **Single admission point**: every agent turn goes through the Governor before reaching a provider. Nothing bypasses it.
2. **Signals**: the Usage collector records per-turn tokens and every plan-limit signal the provider exposes (utilization, reset times, rate-limit events). Where utilization is not exposed, an **Estimator** converts tokens to "% of window" using per-model weights, calibrated against observed signals and limit hits.
3. **Policy**: the Weekly plan defines a working calendar, allocations (share %, priority) and a reserve. From it we compute an **ideal cumulative curve**. The Governor compares actual usage with the curve (weekly and 5h windows) and escalates through levels L1–L7 (queue low priority → cap concurrency → smooth 5h peaks → downgrade model/effort → compact context → pause teams at checkpoints → defer after reset). Each level is set to *auto / ask / off*.
4. **Protection**: interactive sessions and *critical* allocations are never throttled unless the user opts in.
5. **Transparency**: every decision is a `GovernorAction` record shown in the pacing log, and "ask" actions become Inbox items.

## Open point
- The exact signals the Claude Agent SDK exposes for subscription plans must be verified in the P0 spike (SPEC R1). The design works with estimator-only data, at lower accuracy.

## Consequences
- Pausing, prioritizing and accounting come for free from the single admission path.
- Slight latency when turns are queued, by design.
