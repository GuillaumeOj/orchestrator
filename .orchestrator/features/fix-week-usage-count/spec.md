# Fix Week Usage Count · Feature spec

> Feature `fix-week-usage-count` · Integration branch `orch/fix-week-usage-count/integration` · Core only (`agent-runtime`, `governor`, `orch-store`) · No UI change, no design pass

## 1. Problem

The week total (`W`, the account's weekly %) is right. The split is wrong: most of `W` shows as **outside the app** (title-bar week bar, Planner window, weekly report), even when nearly all the work ran in the app, in solo sessions and in feature teams.

The split is computed, not observed. The statusline gives the weekly % and each session's cumulative cost (`cost.total_cost_usd`). The Governor fits two numbers over the intervals between readings, `ΔW ≈ Δcost / k + u · Δhours` (`crates/governor/src/estimator.rs`):

- agents and you = `cost / k`
- outside the app = `W − app cost / k` (`governor.rs` `metrics`, `report.rs` `build`)

So when `k` (cost per 1 % of the week) comes out too high, every app row shrinks and "outside the app" takes the rest.

## 2. Root cause

### 2.1 The app double-counts a resumed session's cost after each core restart

- `UsageCollector` (`crates/agent-runtime/src/usage.rs`) keeps each agent's last `total_cost_usd` **in memory only**, and turns each render into a delta: `cost − last`.
- When `claude --resume <id>` starts, Claude Code **restores that session's cost** (`restoreCostStateForSession`, checked in the 2.1.326 binary: `lastCost` and the other session counters are saved per project and put back on resume). So the first render of a resumed process reports the session's whole cost so far.
- After a core restart (quit, crash, and every **auto-update**: 0.1.225 → 0.1.227 this week), the collector is empty, the reconciler resumes the live agents, and the first sample of each one carries `delta = total_cost_usd`: the session's whole past cost, counted a second time. For a lead session that runs for a whole feature, that is the cost of hours of work, in one sample.
- The same applies to any first sample whose session the collector does not know (`new_session` ⇒ `delta = cost`) when that session was resumed rather than started fresh.

### 2.2 One inflated interval moves `k` a lot, and for a long time

The fit is unweighted least squares on raw cost, with decay 0.85 per interval. An interval with a cost spike and an ordinary `ΔW` (3 pp) has large leverage. It pulls `1/k` down, and `u` (pp per wall hour) absorbs the normal intervals. A simulation of a week run by agents only (true `k` = 20 $/pp, true outside = 6.6 pp of 95):

| double-counted restarts in the week | fitted `k` | app | outside |
|---|---|---|---|
| none | 19.9 | 89 pp | 6 pp |
| one late in the week | 35.0 | 62 pp | **33 pp** |

Several restarts a day keep `k` high. Nothing in the fit rejects an interval whose app cost alone explains several times the observed increase, which cannot happen.

The stored rows keep the inflated `cost_delta`, so a restart replays the week (`restore_governor`) with the same spikes, and the weekly report (`report.rs`) is built from them.

## 3. Scope

### 3.1 Exact cost deltas across restarts (`agent-runtime`, `orch-store`)

1. Store the statusline `session_id` on each usage row: a new nullable `session_id` column in `usage`, written by `usage_record`. This needs a migration.
2. At startup, seed the collector with each agent's baseline: its current session id (the agent record) and the last `cost_estimate` stored for that agent and session.
3. Delta rule, for every sample:
   - same session as the baseline: `cost ≥ baseline` ⇒ `cost − baseline`; `cost < baseline` (a process that did not restore its cost) ⇒ `cost`, as today;
   - a session the collector has no baseline for: when the store has a last `cost_estimate` for that agent and session (a resume), the same rule against it; else (a new session) `cost`.
4. The core restart path, the idle suspend/resume path and the model-switch resume path all get exact deltas. A fresh session per task (improve-agent-usage) is unchanged: its first cost is its own.

### 3.2 Repair the stored rows (`orch-store` migration, or a startup pass)

5. Rows recorded before the fix with `cost_delta == cost_estimate` (the "first sample" case) while the same agent's previous row has `cost_estimate ≤` this one: set `cost_delta = cost_estimate − previous cost_estimate`. Rows of an agent are ordered by `ts_ms`. The pass runs once (an idempotent migration).
6. So the current week's replay and its in-progress report are right after the update. Stored reports of past weeks are not rebuilt.

### 3.3 A fit that a bad interval cannot capture (`governor`)

7. `Calibrator::update` (and `FamilyCalibrator::update` with the same rule) bounds an interval's influence once the fit has some confidence (for example ≥ 3 real intervals). When `Δcost / k > m · ΔW + slack` (app cost alone explains far more than the increase, with `m` around 2 and `slack` around 1 pp for whole-percent readings), the interval is clipped to `Δcost = k · (m · ΔW + slack)`, or skipped. Back picks the option and its constants, with a test for each. Early in the week (the prior `k0 = 1` is far off), nothing is clipped.
8. The governor's replay goes through the same `update`, so a restart gives the same `k` as the live run.

### 3.4 Out of scope

- UI changes: the bars, Planner window and report already show `governed`, `interactive_used` and `unattributed_so_far` / `outside_pp`. They are right once the inputs are.
- Rebuilding stored weekly reports of past weeks.
- Attribution of cloud work (`/code-review ultra`, remote sessions). It runs outside the app and stays "outside the app".

## 4. Acceptance

1. `usage.rs` unit tests:
   - a collector seeded with `(session s, 12.0)`, then a render of `s` at 12.4 ⇒ delta 0.4;
   - the same render at 0.3 (cost not restored) ⇒ 0.3;
   - an unseeded collector with the store's baseline for `s` at 12.0, then a render of `s` at 12.4 ⇒ 0.4;
   - a new session `t` at 0.2 ⇒ 0.2.
2. Runtime test (the existing restart harness in `runtime.rs` / `tests/e2e_fake_claude.rs`): an agent reports cost 5.0, the core restarts, the fake claude resumes and reports 5.0, then 5.5. Stored deltas sum to 5.5, not 10.5.
3. Migration test: rows `(a, 1.0, Δ1.0) (a, 3.0, Δ2.0) (a, 3.4, Δ3.4)` ⇒ the last delta becomes 0.4. A real new session `(a, 0.2, Δ0.2)` after `3.4` is left as it is.
4. Estimator test: the §2.2 week (agents only, true `k` 20, outside 6.6 pp) with three double-counted intervals keeps `k` within 10 % of 20, and the outside estimate within 3 pp of the truth. The existing `learns_k_and_u_from_mixed_intervals` and the pacing-sim NFR tests still pass.
5. Governor test: live ingest and restart replay of the same samples give the same `k` and the same `unattributed_so_far`.
6. `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check` pass.

## 5. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-A · Exact cost deltas across restarts + row repair (§3.1, §3.2) | back | `usage.rs`, `runtime.rs` (`on_status`, startup, `usage_record`), `orch-store` (migration, `UsageRecord.session_id`, a "last cost of agent/session" query). Opus. |
| T-B · Bounded calibration intervals (§3.3) | back | `crates/governor/src/estimator.rs`, `governor.rs` `calibrate`. No file in common with T-A. |

Front has no task: the UI does not change.
