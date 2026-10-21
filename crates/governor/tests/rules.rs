//! One test (or a few) per rule of ADR-007 / SPEC §4.8, through the public API.

mod common;

use common::*;
use governor::*;

// ---------------------------------------------------------------- admission & caps

#[test]
fn green_admits_up_to_base_then_queues_on_the_cap() {
    let mut g = governor();
    for a in ["A-1", "A-2", "B-1"] {
        assert_eq!(g.request_turn(task(a, &a[..1], Tier::Normal), at(0)).effective, Verdict::Admit);
    }
    let d = g.request_turn(task("C-1", "C", Tier::Low), at(0));
    assert_eq!(queue_reason(&d), Some(QueueReason::LiveCap { state: PressureState::Green, cap: 3 }));
    assert_eq!(g.live_autonomous(), 3);
    assert_eq!(g.queue().len(), 1);
    assert_eq!(count(&g, "queue"), 1);
}

#[test]
fn interactive_work_is_never_blocked() {
    let mut g = governor();
    g.pause_all(at(0));
    g.rate_limit_hit(None, Window::Weekly, at(600), at(1));
    let mut r = task("user-pane", "A", Tier::Low);
    r.interactive = true;
    assert_eq!(g.request_turn(r, at(2)).effective, Verdict::Admit);
    let t = ToolCallRequest { agent: "user-pane".into(), interactive: true };
    assert_eq!(g.tool_call(&t, at(3)).effective, Verdict::Admit);
    // An agent the Governor never admitted, typed into by the user.
    let t = ToolCallRequest { agent: "other".into(), interactive: true };
    assert_eq!(g.tool_call(&t, at(3)).effective, Verdict::Admit);
}

#[test]
fn critical_work_bypasses_the_cap_and_red() {
    let mut g = governor();
    for a in ["A-1", "A-2", "B-1"] {
        g.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    assert_eq!(g.request_turn(task("A-9", "A", Tier::Critical), at(0)).effective, Verdict::Admit);
    assert_eq!(g.live_autonomous(), 3, "critical turns are not counted against the cap");
    drive_red(&mut g, 40);
    assert_eq!(g.request_turn(task("A-8", "A", Tier::Critical), at(41)).effective, Verdict::Admit);
}

#[test]
fn caps_follow_the_state() {
    let mut g = governor();
    drive_red(&mut g, 40);
    assert_eq!(g.snapshot(at(40)).live_cap, 1);
    assert_eq!(g.request_turn(task("A-1", "A", Tier::High), at(41)).effective, Verdict::Admit);
    let d = g.request_turn(task("A-2", "A", Tier::High), at(41));
    assert_eq!(queue_reason(&d), Some(QueueReason::LiveCap { state: PressureState::Red, cap: 1 }));
}

#[test]
fn amber_halves_the_cap() {
    let mut g = governor();
    // Plan at Mon 12:00 is 3 h / 50 h × 90 ≈ 5.4 %; read 9 % → e ≈ +4.1.
    g.ingest(&weekly(at(180), "obs", 90.0, 9.0));
    g.tick(at(180));
    assert_eq!(g.state(), PressureState::Amber);
    assert_eq!(g.snapshot(at(180)).live_cap, 2);
}

#[test]
fn red_queues_low_tier_and_admits_normal() {
    let mut g = governor();
    drive_red(&mut g, 40);
    let d = g.request_turn(task("C-1", "C", Tier::Low), at(41));
    assert_eq!(queue_reason(&d), Some(QueueReason::RedTier));
    assert_eq!(g.request_turn(task("B-1", "B", Tier::Normal), at(41)).effective, Verdict::Admit);
}

#[test]
fn red_estimated_start_is_bounded_by_aging() {
    let mut g = governor();
    drive_red(&mut g, 40);
    g.request_turn(task("C-1", "C", Tier::Low), at(41));
    let e = &g.queue()[0];
    let start = e.estimated_start.expect("estimated");
    assert!(start <= at(41 + 120), "{start}");
    assert!(start >= at(41));
}

#[test]
fn off_hours_without_allowance_queue_until_the_next_working_time() {
    let mut g = governor();
    let evening = at(hours(11.0)); // Mon 20:00 Paris
    g.tick(evening);
    let d = g.request_turn(task("A-1", "A", Tier::High), evening);
    assert_eq!(queue_reason(&d), Some(QueueReason::OffHours));
    let tue_9 = at(hours(24.0));
    assert_eq!(g.queue()[0].estimated_start, Some(tue_9));
    assert!(g.tick(at(hours(23.0))).is_empty());
    let released = g.tick(tue_9);
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].agent, AgentId::from("A-1"));
}

#[test]
fn off_hours_allowance_lets_work_run_at_night() {
    let mut p = plan();
    p.off_hours_allowance_pct = 20.0;
    let mut g = Governor::new(p, policy(), start()).unwrap();
    let evening = at(hours(11.0));
    g.tick(evening);
    assert_eq!(g.request_turn(task("A-1", "A", Tier::High), evening).effective, Verdict::Admit);
}

#[test]
fn caps_scale_with_the_block_weight() {
    let mut p = plan();
    for b in &mut p.calendar.blocks {
        b.weight = 0.5;
    }
    let mut g = Governor::new(p, policy(), start()).unwrap();
    g.tick(at(10));
    assert_eq!(g.snapshot(at(10)).live_cap, 2, "ceil(3 × 0.5)");
}

#[test]
fn hard_cap_per_allocation() {
    let mut p = plan();
    p.allocations[1].hard_cap_pct = Some(5.0);
    let mut g = Governor::new(p, policy(), start()).unwrap();
    g.request_turn(task("B-1", "B", Tier::Normal), at(0));
    g.ingest(&UsageSample::cost(at(30), "B-1".into(), 60.0)); // 6 pp at k = 10
    g.turn_ended(&"B-1".into(), at(31));
    let d = g.request_turn(task("B-1", "B", Tier::Normal), at(32));
    assert_eq!(queue_reason(&d), Some(QueueReason::HardCap));
    assert_eq!(g.queue()[0].estimated_start, Some(RESET.parse().unwrap()));
}

// ---------------------------------------------------------------- fair queuing

/// Keep both allocations backlogged with cap 1 and count who gets the slot.
fn wfq_shares(weights: (f64, f64), turns: usize) -> (usize, usize) {
    let mut p = plan();
    p.allocations = vec![alloc("X", weights.0, Tier::Normal), alloc("Y", weights.1, Tier::Normal)];
    let mut pol = policy();
    pol.live_caps.green = 1;
    let mut g = Governor::new(p, pol, start()).unwrap();
    let agents = ["X-1", "X-2", "Y-1", "Y-2"];
    for a in agents {
        g.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    let (mut x, mut y) = (0, 0);
    for n in 0..turns {
        let now = at(1 + n as i64 * 5);
        let live = agents.iter().find(|a| g.is_live(&AgentId::from(**a))).expect("one live");
        if live.starts_with('X') {
            x += 1
        } else {
            y += 1
        }
        g.ingest(&UsageSample::cost(now, (*live).into(), 5.0));
        let released = g.turn_ended(&(*live).into(), now);
        assert_eq!(released.len(), 1);
        g.request_turn(task(live, &live[..1], Tier::Normal), now);
    }
    (x, y)
}

#[test]
fn weighted_fair_queuing_follows_weights() {
    let (x, y) = wfq_shares((3.0, 1.0), 80);
    assert!((58..=62).contains(&x), "x={x} y={y}");
    let (x, y) = wfq_shares((1.0, 1.0), 80);
    assert!((38..=42).contains(&x), "x={x} y={y}");
}

#[test]
fn unused_weight_is_redistributed() {
    let mut g = governor();
    // Only A has work: it gets every slot even though its weight is 50 %.
    for a in ["A-1", "A-2", "A-3", "A-4"] {
        g.request_turn(task(a, "A", Tier::Normal), at(0));
    }
    assert_eq!(g.live_autonomous(), 3);
    let r = g.turn_ended(&"A-1".into(), at(10));
    assert_eq!(r[0].agent, AgentId::from("A-4"));
}

#[test]
fn an_idle_allocation_does_not_bank_credit() {
    let mut p = plan();
    p.allocations = vec![alloc("X", 1.0, Tier::Normal), alloc("Y", 1.0, Tier::Normal)];
    let mut pol = policy();
    pol.live_caps.green = 1;
    let mut g = Governor::new(p, pol, start()).unwrap();
    // X works alone for a long time.
    g.request_turn(task("X-1", "X", Tier::Normal), at(0));
    for n in 0..20 {
        g.ingest(&UsageSample::cost(at(n * 10 + 5), "X-1".into(), 10.0));
        g.turn_ended(&"X-1".into(), at(n * 10 + 9));
        g.request_turn(task("X-1", "X", Tier::Normal), at(n * 10 + 9));
    }
    // Now Y wakes up with work: it must not get the next 20 slots in a row.
    g.request_turn(task("Y-1", "Y", Tier::Normal), at(300));
    g.request_turn(task("Y-2", "Y", Tier::Normal), at(300));
    g.request_turn(task("X-2", "X", Tier::Normal), at(300));
    let mut order = Vec::new();
    for n in 0..6 {
        let now = at(301 + n * 10);
        let live: AgentId =
            ["X-1", "X-2", "Y-1", "Y-2"].iter().map(|a| AgentId::from(*a)).find(|a| g.is_live(a)).unwrap();
        g.ingest(&UsageSample::cost(now, live.clone(), 10.0));
        let r = g.turn_ended(&live, now);
        order.push(r[0].allocation.0.clone());
        g.request_turn(task(&live.0, &live.0[..1], Tier::Normal), now);
    }
    assert!(order.iter().any(|a| a == "X"), "X starved after Y woke up: {order:?}");
}

#[test]
fn strict_priority_for_critical_and_tiers() {
    let mut g = governor();
    g.pause_all(at(0));
    g.request_turn(task("C-1", "C", Tier::Low), at(1));
    g.request_turn(task("B-1", "B", Tier::Normal), at(2));
    g.request_turn(task("A-1", "A", Tier::High), at(3));
    g.request_turn(task("A-9", "A", Tier::Critical), at(4));
    let mut pol_order = Vec::new();
    for r in g.resume_all(at(5)) {
        pol_order.push(r.agent.0);
    }
    assert_eq!(pol_order, vec!["A-9", "A-1", "B-1", "C-1"]);
}

#[test]
fn aging_prevents_starvation() {
    let mut pol = policy();
    pol.live_caps.green = 1;
    let mut g = Governor::new(plan(), pol, start()).unwrap();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    g.request_turn(task("C-1", "C", Tier::Low), at(0));
    // A steady flood of high-tier work: one more queued every turn.
    let mut released_low = None;
    for n in 1..60 {
        let now = at(n * 20);
        g.request_turn(task(&format!("A-{}", n + 1), "A", Tier::High), now);
        let live = g.queue().len(); // keep the queue non-empty
        assert!(live > 0);
        let running: Vec<AgentId> = (0..=n + 1)
            .map(|i| AgentId(format!("A-{i}")))
            .chain([AgentId::from("C-1")])
            .filter(|a| g.is_live(a))
            .collect();
        for a in running {
            g.ingest(&UsageSample::cost(now, a.clone(), 3.0));
            for r in g.turn_ended(&a, now) {
                if r.agent.0 == "C-1" {
                    released_low = Some(n * 20);
                }
            }
        }
        if released_low.is_some() {
            break;
        }
    }
    let when = released_low.expect("the low-tier turn must run");
    assert!(when <= 4 * 60 + 20, "released after {when} min");
    assert!(when >= 2 * 60, "aging should take hours, not minutes ({when} min)");
}

// ---------------------------------------------------------------- measurements

#[test]
fn w_uses_the_most_recent_sample_across_agents() {
    let mut g = governor();
    g.ingest(&weekly(at(60), "A-1", 400.0, 40.0));
    // An older response from another agent arrives later: ignored.
    g.ingest(&weekly(at(50), "B-1", 0.0, 55.0));
    assert!((g.snapshot(at(61)).used_central - 40.5).abs() < 1e-9);
    // A newer one wins even if lower (per-process copies, FR-801).
    g.ingest(&weekly(at(70), "B-1", 0.0, 38.0));
    assert!((g.snapshot(at(71)).used_central - 38.5).abs() < 1e-9);
}

#[test]
fn a_missing_window_is_unknown_not_zero() {
    let mut g = governor();
    g.ingest(&weekly(at(60), "A-1", 100.0, 10.0));
    let s = g.snapshot(at(61));
    assert_eq!(s.five_hour, None);
    assert_eq!(s.five_hour_projected, None);
    assert!(!s.five_hour_guard);
}

#[test]
fn stale_observation_becomes_an_estimate_with_margin() {
    let mut g = governor();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    g.ingest(&weekly(at(60), "A-1", 200.0, 20.0));
    assert_eq!(g.snapshot(at(70)).provenance, Provenance::Observed);
    g.ingest(&UsageSample::cost(at(80), "A-1".into(), 50.0)); // +5 pp at k = 10
    let s = g.snapshot(at(90));
    assert_eq!(s.provenance, Provenance::Estimated);
    assert!((s.used_central - 25.5).abs() < 1e-9, "{}", s.used_central);
    assert!((s.margin - 5.0).abs() < 1e-9, "margin capped by the estimated delta: {}", s.margin);
    assert!((s.used - 30.5).abs() < 1e-9);
}

#[test]
fn full_margin_without_any_anchor() {
    // Created mid-week with no observation: W is unknown, so the full margin applies.
    let g = Governor::new(plan(), policy(), at(hours(30.0))).unwrap();
    let s = g.snapshot(at(hours(30.0)));
    assert_eq!(s.provenance, Provenance::Estimated);
    assert!((s.margin - 15.0).abs() < 1e-9, "{}", s.margin);
}

#[test]
fn estimator_learns_k_and_unattributed_usage() {
    let mut g = governor(); // prior k = 10
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    // Truth: k = 8 cost/pp; 0.3 pp/h outside the app, day and night.
    // Agents report hourly during working time only.
    let mut w: f64 = 0.0;
    let mut last = 0;
    let mut end = 0;
    for day in 0..4 {
        for h in 1..=10 {
            let minute = day * 24 * 60 + h * 60;
            let app_pp = 0.8 + 0.5 * (h % 3) as f64;
            w += app_pp + 0.3 * (minute - last) as f64 / 60.0;
            last = minute;
            end = minute;
            g.ingest(&weekly(at(minute), "A-1", app_pp * 8.0, w.floor()));
            g.tick(at(minute));
        }
    }
    let c = g.calibration(at(end));
    assert!((c.cost_per_weekly_pct - 8.0).abs() < 0.8, "k = {}", c.cost_per_weekly_pct);
    assert!((c.unattributed_rate - 0.3).abs() < 0.1, "u = {}", c.unattributed_rate);
    let s = g.snapshot(at(end));
    assert!(s.unattributed_so_far > 10.0, "{s:?}");
    assert!(s.unattributed_forecast > s.unattributed_so_far);
    assert!(s.plan > s.ideal, "unattributed usage so far is part of the plan");
}

#[test]
fn ideal_curve_follows_weighted_hours() {
    let g = governor();
    // Tue 09:00 = 10 of 50 h; ideal = 10/50 × (100 − 10) = 18.
    let s = g.snapshot(at(hours(24.0)));
    assert!((s.ideal - 18.0).abs() < 1e-6, "{}", s.ideal);
    // The night does not move it.
    let night = g.snapshot(at(hours(20.0)));
    assert!((night.ideal - 18.0).abs() < 1e-6);
    assert!((g.snapshot(RESET.parse().unwrap()).ideal - 90.0).abs() < 1e-6);
}

#[test]
fn eta_projects_the_burn_over_working_time() {
    let mut g = governor();
    // Burn 4 pp per working hour from the start: 100 % in 25 h, i.e. Wednesday.
    for m in (0..=hours(4.0)).step_by(10) {
        g.ingest(&weekly(at(m), "A-1", 0.0, (m as f64 / 15.0).floor()));
        g.tick(at(m));
    }
    let s = g.snapshot(at(hours(4.0)));
    let eta = s.eta.expect("projected");
    assert!(eta < RESET.parse().unwrap());
    assert_eq!(g.state(), PressureState::Red, "{s:?}");
}

// ---------------------------------------------------------------- 5h guard

#[test]
fn five_hour_guard_holds_autonomous_work_above_threshold() {
    let mut g = governor();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    g.ingest(&five(at(10), "A-1", 86.0, at(100)));
    let d = g.tool_call(&tool("A-1"), at(11));
    assert_eq!(
        d.effective,
        Verdict::Deny { reason: DenyReason::PausedByGovernor { cause: QueueReason::FiveHourGuard } }
    );
    assert!(!g.is_live(&"A-1".into()));
    assert_eq!(g.queue()[0].estimated_start, Some(at(100)), "resumes at the 5h reset");
    let d = g.request_turn(task("B-1", "B", Tier::Normal), at(12));
    assert_eq!(queue_reason(&d), Some(QueueReason::FiveHourGuard));
    assert_eq!(g.request_turn(task("A-9", "A", Tier::Critical), at(12)).effective, Verdict::Admit);
    assert_eq!(g.tool_call(&tool("A-9"), at(13)).effective, Verdict::Admit);
    // After the 5h reset the queue runs again.
    let r = g.tick(at(100));
    assert!(!r.is_empty());
}

#[test]
fn five_hour_projection_caps_live_agents_to_one() {
    let mut g = governor();
    for (m, f) in [(0, 0.0), (10, 10.0), (20, 20.0), (30, 30.0), (40, 40.0), (50, 50.0)] {
        g.ingest(&five(at(m), "A-1", f, at(300)));
        g.tick(at(m));
    }
    let s = g.snapshot(at(50));
    assert!(s.five_hour_guard, "{s:?}");
    assert_eq!(s.live_cap, 1);
    assert_eq!(count(&g, "five_hour_guard"), 1);
}

// ---------------------------------------------------------------- pressure, asks

#[test]
fn state_changes_wait_for_the_minimum_dwell() {
    let mut g = governor();
    g.ingest(&weekly(at(5), "obs", 0.0, 30.0));
    for m in 5..30 {
        g.tick(at(m));
        assert_eq!(g.state(), PressureState::Green, "dwell from creation at minute {m}");
    }
    g.tick(at(30));
    assert_eq!(g.state(), PressureState::Red);
    let preview = g.next_action_preview(at(31));
    assert_eq!(preview.earliest_state_change, at(60));
}

#[test]
fn entering_red_asks_to_defer_low_tier_work_and_applies_the_default() {
    let mut g = governor();
    g.request_turn(task("C-1", "C", Tier::Low), at(0));
    drive_red(&mut g, 40);
    let ask = g.actions().iter().find(|a| a.kind_name() == "defer_after_reset").expect("ask").clone();
    assert_eq!(ask.mode, ActionMode::Ask);
    assert_eq!(ask.outcome, Outcome::Pending);
    assert_eq!(ask.ask.as_ref().unwrap().deadline, at(60));
    g.tick(at(59));
    assert_eq!(g.action(ask.id).unwrap().outcome, Outcome::Pending);
    g.tick(at(60));
    assert_eq!(g.action(ask.id).unwrap().outcome, Outcome::Applied);
    g.turn_ended(&"C-1".into(), at(61));
    let d = g.request_turn(task("C-1", "C", Tier::Low), at(62));
    assert_eq!(queue_reason(&d), Some(QueueReason::DeferredUntilReset));
    // Undo lifts the deferral.
    g.undo(ask.id, at(63)).unwrap();
    g.tick(at(64));
    assert_ne!(g.queue()[0].reason, QueueReason::DeferredUntilReset);
}

#[test]
fn a_declined_ask_is_skipped() {
    let mut g = governor();
    g.request_turn(task("C-1", "C", Tier::Low), at(0));
    drive_red(&mut g, 40);
    let id = g.next_action_preview(at(41)).pending_ask.expect("pending").0;
    g.answer_ask(id, false, at(41)).unwrap();
    assert_eq!(g.action(id).unwrap().outcome, Outcome::Skipped);
    assert!(g.answer_ask(id, true, at(42)).is_err());
}

#[test]
fn ask_default_can_be_skip() {
    let mut pol = policy();
    pol.default_ask_action = AskDefault::Skip;
    let mut g = Governor::new(plan(), pol, start()).unwrap();
    g.request_turn(task("C-1", "C", Tier::Low), at(0));
    drive_red(&mut g, 40);
    g.tick(at(61));
    let a = g.actions().iter().find(|a| a.kind_name() == "defer_after_reset").unwrap();
    assert_eq!(a.outcome, Outcome::Skipped);
}

#[test]
fn every_action_has_a_reason_and_an_expected_effect() {
    let mut g = governor();
    g.request_turn(task("C-1", "C", Tier::Low), at(0));
    for a in ["A-1", "A-2", "B-1"] {
        g.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    drive_red(&mut g, 40);
    g.tick(at(80));
    g.pause_all(at(81));
    g.tool_call(&tool("A-1"), at(82));
    assert!(g.actions().len() >= 5);
    for a in g.actions() {
        assert!(!a.reason.is_empty() && !a.expected_effect.is_empty(), "{a:?}");
    }
}

// ---------------------------------------------------------------- sprint

#[test]
fn sprint_borrows_from_the_rest_of_the_week() {
    let mut g = governor();
    let base = governor();
    let wed_10 = at(hours(49.0));
    let wed_19 = at(hours(58.0));
    let id = g.start_sprint(4.0, wed_10).unwrap();
    let (_, until, extra) = g.sprint().unwrap();
    assert_eq!(until, wed_19);
    assert!((extra - 4.0).abs() < 1e-6);
    let d = g.snapshot(wed_19).ideal - base.snapshot(wed_19).ideal;
    assert!((d - 4.0).abs() < 1e-6, "{d}");
    assert!(g.snapshot(wed_10).ideal == base.snapshot(wed_10).ideal);
    let reset = RESET.parse().unwrap();
    assert!((g.snapshot(reset).ideal - base.snapshot(reset).ideal).abs() < 1e-6, "nothing extra for the week");
    g.undo(id, wed_19).unwrap();
    assert!((g.snapshot(wed_19).ideal - base.snapshot(wed_19).ideal).abs() < 1e-9);
    assert!(g.undo(id, wed_19).is_err());
}

#[test]
fn sprint_needs_working_time_today() {
    let mut g = governor();
    assert_eq!(g.start_sprint(4.0, at(hours(11.0))), Err(GovernorError::NoWorkingTimeToday));
}

// ---------------------------------------------------------------- shadow mode

#[test]
fn shadow_mode_decides_and_logs_but_never_enforces() {
    let mut p = plan();
    p.shadow_mode = true;
    let mut g = Governor::new(p, policy(), start()).unwrap();
    for a in ["A-1", "A-2", "B-1"] {
        g.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    let d = g.request_turn(task("C-1", "C", Tier::Low), at(0));
    assert_eq!(d.effective, Verdict::Admit);
    assert!(matches!(d.proposed, Verdict::Queue { entry: None, reason: QueueReason::LiveCap { .. }, .. }));
    assert!(g.queue().is_empty());
    g.pause_all(at(1));
    let d = g.tool_call(&tool("A-1"), at(2));
    assert_eq!(d.effective, Verdict::Admit);
    assert!(matches!(d.proposed, Verdict::Deny { .. }));
    assert!(g.is_live(&"A-1".into()));
    assert!(g.actions().iter().any(|a| a.mode == ActionMode::Shadow));
    assert_eq!(g.stats().shadow_overrides, 2);
    drive_red_shadow(&mut g);
}

fn drive_red_shadow(g: &mut Governor) {
    g.ingest(&weekly(at(40), "obs", 0.0, 30.0));
    g.tick(at(40));
    assert_eq!(g.state(), PressureState::Red, "shadow mode still tracks the state");
    let defer = g.actions().iter().find(|a| a.kind_name() == "defer_after_reset");
    if let Some(a) = defer {
        assert_eq!(a.mode, ActionMode::Shadow);
    }
}

// ---------------------------------------------------------------- FR-410, J9, watchdog

#[test]
fn self_started_turns_are_counted_and_held_at_tool_calls() {
    let mut g = governor();
    for a in ["A-1", "A-2", "B-1"] {
        g.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    let mut r = task("C-1", "C", Tier::Low);
    r.self_started = true;
    r.kind = TurnKind::Continue;
    let d = g.request_turn(r, at(5));
    assert!(queue_reason(&d).is_some());
    assert!(g.queue()[0].self_started);
    let t = g.tool_call(&tool("C-1"), at(6));
    assert!(matches!(t.effective, Verdict::Deny { reason: DenyReason::PausedByGovernor { .. } }));
    let released = g.turn_ended(&"A-1".into(), at(10));
    assert_eq!(released[0].agent, AgentId::from("C-1"));
    assert!(released[0].self_started);
    g.ingest(&UsageSample::cost(at(20), "C-1".into(), 15.0));
    assert!((g.allocation_usage_pct(&"C".into()) - 1.5).abs() < 1e-9);
}

#[test]
fn a_self_started_turn_with_a_free_slot_is_admitted_and_counted() {
    let mut g = governor();
    let mut r = task("B-1", "B", Tier::Normal);
    r.self_started = true;
    assert_eq!(g.request_turn(r, at(0)).effective, Verdict::Admit);
    assert_eq!(g.live_autonomous(), 1);
}

#[test]
fn rate_limit_cancels_auto_continue_and_resumes_at_the_reset() {
    let mut g = governor();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    g.request_turn(task("B-1", "B", Tier::Normal), at(0));
    let reset = at(120);
    let resp = g.rate_limit_hit(Some(&"A-1".into()), Window::FiveHour, reset, at(30));
    assert!(resp.cancel_auto_continue);
    let entry = resp.entry.expect("rescheduled");
    assert!(!g.is_live(&"A-1".into()));
    assert_eq!(count(&g, "limit_reached"), 1);
    assert_eq!(count(&g, "cancel_auto_continue"), 1);
    // Other agents stop at their next tool call.
    let d = g.tool_call(&tool("B-1"), at(31));
    assert_eq!(
        d.effective,
        Verdict::Deny {
            reason: DenyReason::PausedByGovernor { cause: QueueReason::LimitReached { window: Window::FiveHour } }
        }
    );
    let d = g.request_turn(task("C-1", "C", Tier::Low), at(32));
    assert_eq!(queue_reason(&d), Some(QueueReason::LimitReached { window: Window::FiveHour }));
    g.tick(at(33));
    assert!(g.queue().iter().all(|e| e.estimated_start == Some(reset)));
    assert!(g.tick(at(119)).is_empty());
    let released = g.tick(reset);
    assert_eq!(count(&g, "limit_reset"), 1);
    // Priority order: A (high) first.
    assert_eq!(released[0].entry, entry);
    assert_eq!(released.len(), 3);
}

#[test]
fn auto_continue_can_be_allowed() {
    let mut pol = policy();
    pol.auto_continue = AutoContinue::Allow;
    let mut g = Governor::new(plan(), pol, start()).unwrap();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    let resp = g.rate_limit_hit(Some(&"A-1".into()), Window::FiveHour, at(120), at(30));
    assert!(!resp.cancel_auto_continue);
    assert_eq!(resp.entry, None);
}

#[test]
fn a_100_percent_reading_counts_as_a_limit() {
    let mut g = governor();
    g.ingest(&five(at(10), "A-1", 100.0, at(200)));
    let d = g.request_turn(task("A-1", "A", Tier::High), at(11));
    assert_eq!(queue_reason(&d), Some(QueueReason::LimitReached { window: Window::FiveHour }));
}

#[test]
fn watchdog_interrupts_turns_over_their_ceiling() {
    let mut g = governor();
    let mut r = task("A-1", "A", Tier::High);
    r.ceiling = Some(TurnCeiling { max_pct: Some(2.0), max_minutes: Some(60) });
    g.request_turn(r.clone(), at(0));
    assert_eq!(g.tool_call(&tool("A-1"), at(30)).effective, Verdict::Admit);
    assert_eq!(g.tool_call(&tool("A-1"), at(60)).effective, Verdict::Deny { reason: DenyReason::TurnCeiling });
    assert_eq!(count(&g, "watchdog"), 1);
    assert!(!g.is_live(&"A-1".into()));
    // Usage ceiling.
    g.request_turn(r, at(61));
    g.ingest(&UsageSample::cost(at(62), "A-1".into(), 25.0));
    assert_eq!(g.tool_call(&tool("A-1"), at(63)).effective, Verdict::Deny { reason: DenyReason::TurnCeiling });
}

#[test]
fn pause_all_holds_at_the_next_tool_call_and_undo_resumes() {
    let mut g = governor();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    let id = g.pause_all(at(1));
    let d = g.tool_call(&tool("A-1"), at(2));
    assert_eq!(d.effective, Verdict::Deny { reason: DenyReason::PausedByGovernor { cause: QueueReason::PausedAll } });
    assert_eq!(d.effective.to_string_reason(), "paused by governor (paused by the user)");
    assert_eq!(g.queue()[0].reason, QueueReason::PausedAll);
    let r = g.undo(id, at(3)).unwrap();
    assert_eq!(r.len(), 1);
}

trait ReasonText {
    fn to_string_reason(&self) -> String;
}

impl ReasonText for Verdict {
    fn to_string_reason(&self) -> String {
        match self {
            Verdict::Deny { reason } => reason.to_string(),
            other => format!("{other:?}"),
        }
    }
}

#[test]
fn run_now_overrides_the_queue() {
    let mut g = governor();
    g.pause_all(at(0));
    g.request_turn(task("C-1", "C", Tier::Low), at(1));
    let e = g.queue()[0].id;
    let r = g.run_now(e, at(2)).unwrap();
    assert_eq!(r.agent, AgentId::from("C-1"));
    assert!(g.is_live(&"C-1".into()));
    assert!(g.queue().is_empty());
    assert_eq!(count(&g, "run_now"), 1);
}

#[test]
fn new_week_resets_budgets_but_keeps_calibration_and_queue() {
    let mut g = governor();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    g.ingest(&weekly(at(60), "A-1", 30.0, 3.0));
    g.pause_all(at(61));
    g.request_turn(task("B-1", "B", Tier::Normal), at(62));
    let next_start: governor::jiff::Timestamp = RESET.parse().unwrap();
    let mut p = plan();
    p.week_start = next_start;
    p.reset_at = "2026-11-02T08:00:00Z".parse().unwrap();
    g.new_week(p, next_start).unwrap();
    assert_eq!(g.allocation_usage_pct(&"A".into()), 0.0);
    assert_eq!(g.queue().len(), 1);
    let s = g.snapshot(next_start);
    assert!(s.used_central.abs() < 1e-9);
    assert_eq!(g.state(), PressureState::Green);
}

// ---------------------------------------------------------------- review regressions

#[test]
fn a_turns_last_sample_after_stop_still_counts_as_autonomous() {
    let mut g = governor();
    g.request_turn(task("A-1", "A", Tier::High), at(0));
    g.turn_ended(&"A-1".into(), at(10));
    // The statusline sample of the turn's last response arrives after Stop.
    g.ingest(&UsageSample::cost(at(10), "A-1".into(), 20.0));
    assert_eq!(g.snapshot(at(11)).interactive_used, 0.0);
    // Later usage with no admitted turn is the user's own.
    g.ingest(&UsageSample::cost(at(30), "A-1".into(), 20.0));
    assert!((g.snapshot(at(31)).interactive_used - 2.0).abs() < 1e-9);
}

#[test]
fn critical_work_is_not_stuck_behind_the_agents_queued_entry() {
    let mut g = governor();
    for a in ["A-1", "A-2", "B-1"] {
        g.request_turn(task(a, &a[..1], Tier::Normal), at(0));
    }
    assert!(queue_reason(&g.request_turn(task("B-2", "B", Tier::Normal), at(1))).is_some());
    assert_eq!(g.request_turn(task("B-2", "B", Tier::Critical), at(2)).effective, Verdict::Admit);
    assert!(g.queue().is_empty());
}

#[test]
fn undoing_a_sprint_ends_every_stacked_sprint() {
    let mut g = governor();
    let first = g.start_sprint(2.0, at(hours(25.0))).unwrap(); // Tue 10:00
    let second = g.start_sprint(2.0, at(hours(49.0))).unwrap(); // Wed 10:00
    g.undo(second, at(hours(50.0))).unwrap();
    assert!(g.action(first).unwrap().undone_at.is_some());
    assert_eq!(g.undo(first, at(hours(51.0))), Err(GovernorError::AlreadyUndone(first)));
    assert!((g.snapshot(at(hours(58.0))).ideal - governor().snapshot(at(hours(58.0))).ideal).abs() < 1e-9);
}

#[test]
fn a_sprint_on_the_last_working_day_has_nothing_to_borrow() {
    let mut g = governor();
    let fri_10 = at(hours(97.0));
    assert_eq!(g.start_sprint(4.0, fri_10), Err(GovernorError::NothingToBorrow));
    assert_eq!(count(&g, "sprint"), 0);
}
