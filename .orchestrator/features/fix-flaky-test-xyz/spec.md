# Fix Flaky Test — chaos F6 "the lead's brief done"

## Problem

On `main`, `cargo test -p agent-runtime --lib` sometimes fails (1 of 631) in
`chaos::scenarios::f6_a_member_waiting_for_a_slot_gets_one_from_an_idle_colleague`:

```
panicked at crates/agent-runtime/src/chaos.rs:533:9:
timed out waiting for the lead's brief done
gov: ["Release { entry: EntryId(1), agent: fx-lead-1 } slot free; waited 0 min",
      "Queue { entry: Some(EntryId(1)), agent: fx-lead-1 } waiting behind higher-priority work"]
agents: ["fx-lead-1 Idle flag=None resume_failed=false crashes=0"]
turns: ["8bea9937 Queued attempts=0 body=ORCH: you lead the team … Fix the X bug. …",   # the brief (Task)
        "ed3176bf Done   attempts=0 body=ORCH: approved: proceed. …"]                   # the approval (User)
screens: fx-lead-1: ["DONE[1] [t:ed3176bf] ORCH: approved: proceed. …"]
```

It fails in the shared setup `team_with_lead` (chaos.rs:821), before the F6 body
runs. F6 is the only caller with `base_cap = Some(1)`, so the cap of one live
autonomous agent is what exposes it.

## What the dump says (lead's analysis, not yet proven)

- `sim.team()` → `team::tests::approved` queues the **brief** (`TurnKind::Task`,
  team.rs:503) and then the **approval** (`deliver_user`, `TurnKind::User`,
  team.rs:2163) before the sim loop runs. `attach(LEAD)` then gives the lead a
  live fake CLI that the Governor never admitted (no `request_turn`).
- By priority (`delivery::by_priority`, User 0 < Task 3) the approval is pasted
  first, alone (a `Task` turn still `Pending` never rides along). That much is expected.
- The brief is then never pasted (`attempts=0`) though the lead is `Idle` for
  30 s. The Governor shows one entry for the lead, queued with the default reason
  `BehindQueue` and released at once. That matches the **self-started path of
  `Governor::tool_call`** (governor.rs ~1500): a `PreToolUse` (the fake CLI sends
  one every turn) from an agent the Governor does not count as live enqueues a
  `Continue` entry. It does not match the brief's own `request_turn`.
- `handle_releases` (runtime.rs:2107) then admits whichever queued turn it finds
  (or a governor `Hold`), and something leaves the brief stuck: `Waiting(e)` on a
  gone entry, `Admitted` with a gate that never opens, a `task_boundary` model
  switch that never ends, or a lead with no `Live` while its tracker says `Idle`.
  Background threads (`team_tick`, `watchdog`, `handover_tick`, `suspend`) run
  alongside the sim's `step()`, so this is a race, and it only shows under load
  (CI: 630 tests in 155 s). 40 runs of F6 alone and runs of the whole chaos
  module in parallel did not reproduce it locally.

## Goal

1. **Find the real cause** and say it in the report: which interleaving leaves
   an idle lead's queued brief undelivered.
2. **Fix it where it lives:**
   - a **runtime/Governor bug** (an idle agent's queued turn can stay stuck
     forever in production too): fix the runtime and keep the test as it is;
   - a **test-harness artefact only** (e.g. `attach()` making a live process the
     Governor never admitted, which production never does): fix the harness so
     the setup is deterministic (e.g. let the setup wait for or admit the brief
     the way production does). Do not just raise timeouts.
3. **Regression test:** a deterministic unit test (no sleeps or timing luck)
   that reproduces the interleaving and fails without the fix.
4. **Better failure dump:** `Sim::until` also prints each turn's `admission`,
   whether the agent has a `Live`, its `background_tasks`, `stop_intent`, and the
   agent's last runtime events, so the next flake can be read from CI output.

## Out of scope

- Other flaky tests, unless they share this exact cause (then note them).
- Changing turn priorities (§3.5 F5) or the F6 slot-freeing behaviour.
- UI work.

## Acceptance

- The cause is named in the task report, with the interleaving.
- New regression test fails before the fix and passes after.
- `cargo test -p agent-runtime --lib chaos::` passes repeatedly under parallel
  load (e.g. 8 concurrent runs × 5 rounds) with no F6 failure.
- `cargo clippy` and `cargo fmt --check` clean on the touched crates.

## Team

Backend only (`back`): the code is in `crates/agent-runtime` (runtime.rs,
delivery.rs, chaos.rs) and maybe `crates/governor`. No frontend task.
