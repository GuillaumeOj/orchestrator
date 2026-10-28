//! P3 pacing: per-model weekly windows (FR-815), off-hours scheduling
//! (FR-816) and the opt-in model downgrade (FR-819). Spec §6 items 7 and 8.

mod common;

use common::*;
use governor::*;

fn with_model(agent: &str, allocation: &str, tier: Tier, model: &str) -> TurnRequest {
    TurnRequest { model: Some(model.into()), ..task(agent, allocation, tier) }
}

/// A statusline response carrying `seven_day_opus` at `pct`.
fn opus_window(g: &mut Governor, minutes: i64, pct: f64) {
    g.ingest(&UsageSample {
        model_windows: vec![ModelWindowReading { name: "seven_day_opus".into(), used_pct: pct, resets_at: reset() }],
        ..weekly(at(minutes), "X-1", 0.0, 10.0)
    });
}

#[test]
fn a_model_window_at_95_pct_queues_that_model_only() {
    let mut g = governor();
    g.tick(at(60));
    assert!(g.snapshot(at(60)).model_windows.is_empty(), "nothing shown before a window is seen");
    opus_window(&mut g, 60, 95.0);
    let w = &g.snapshot(at(61)).model_windows[0];
    assert_eq!((w.family.as_str(), w.state), ("opus", ModelWindowState::Red));

    let d = g.request_turn(with_model("A-1", "A", Tier::High, "claude-opus-4-1"), at(61));
    assert_eq!(
        queue_reason(&d),
        Some(QueueReason::ModelWindow { model: "opus".into(), resets_at: reset(), exhausted: false })
    );
    assert_eq!(g.queue()[0].estimated_start, Some(reset()));
    let d = g.request_turn(with_model("B-1", "B", Tier::Normal, "claude-sonnet-4-5"), at(61));
    assert_eq!(d.effective, Verdict::Admit, "sonnet is not held");
    // Critical opus work still runs while the window is only Red; not once it is used up.
    let d = g.request_turn(with_model("C-1", "C", Tier::Critical, "opus"), at(62));
    assert_eq!(d.effective, Verdict::Admit);
    g.model_limit_hit("seven_day_opus", reset(), at(63));
    assert_eq!(g.snapshot(at(63)).model_windows[0].state, ModelWindowState::Exhausted);
    let d = g.request_turn(with_model("D-1", "C", Tier::Critical, "opus"), at(64));
    assert!(matches!(queue_reason(&d), Some(QueueReason::ModelWindow { exhausted: true, .. })));
    // The JSON the planner gets (api-contract §6).
    let v = serde_json::to_value(&g.queue()[0].reason).unwrap();
    assert_eq!(v["kind"], "model_window");
    assert_eq!(v["model"], "opus");
    // A low window is green.
    let mut g = governor();
    opus_window(&mut g, 60, 2.0);
    assert_eq!(g.snapshot(at(61)).model_windows[0].state, ModelWindowState::Green);
}

#[test]
fn downgrade_switches_to_sonnet_at_the_next_task_and_undo_restores() {
    let mut g = governor();
    opus_window(&mut g, 60, 95.0);
    let rule = Downgrade { to: "sonnet".into(), when: DowngradeWhen::ModelWindow };
    let agent = AgentId::from("A-1");
    let m = g.task_boundary_model(&agent, Some("opus"), Some(&rule), at(61));
    assert_eq!(m.as_deref(), Some("sonnet"));
    assert_eq!(g.model_override(&agent).unwrap().note(), "downgraded (model window)");
    let d = g.request_turn(with_model("A-1", "A", Tier::High, "sonnet"), at(61));
    assert_eq!(d.effective, Verdict::Admit, "the downgraded task runs");
    let a = g.actions().iter().rev().find(|a| a.kind_name() == "downgrade").unwrap().clone();
    // Undo: the role's model at the next task, and no new downgrade while the window stays tight.
    g.undo(a.id, at(62)).unwrap();
    assert!(g.model_override(&agent).is_none());
    assert_eq!(g.task_boundary_model(&agent, Some("opus"), Some(&rule), at(63)), None);
    // The window resets: the condition clears and a later squeeze downgrades again.
    let mut g2 = governor();
    opus_window(&mut g2, 60, 95.0);
    assert!(g2.task_boundary_model(&agent, Some("opus"), Some(&rule), at(61)).is_some());
    opus_window(&mut g2, 70, 10.0);
    assert_eq!(g2.task_boundary_model(&agent, Some("opus"), Some(&rule), at(71)), None);
    assert_eq!(count(&g2, "restore_model"), 1);
    // No rule, or a role already on the fallback: nothing.
    assert_eq!(g2.task_boundary_model(&agent, Some("opus"), None, at(72)), None);
    opus_window(&mut g2, 80, 99.0);
    assert_eq!(g2.task_boundary_model(&agent, Some("sonnet"), Some(&rule), at(81)), None);
    // A restart keeps the downgrade (and logs it again, undoable).
    assert!(g2.task_boundary_model(&agent, Some("opus"), Some(&rule), at(82)).is_some());
    let mut g3 = governor();
    g3.restore(g2.carry(), |_| true, at(83));
    assert_eq!(g3.model_override(&agent).map(|o| o.to.as_str()), Some("sonnet"));
}

#[test]
fn work_scheduled_off_hours_waits_for_its_window_then_runs_under_its_cap() {
    let mut g = governor();
    let a1 = AgentId::from("A-1");
    assert_eq!(g.set_off_hours(&a1, true, at(60)), Err(GovernorError::NoOffHoursAllowance));
    let mut p = plan();
    p.off_hours_allowance_pct = 10.0;
    let mut g = Governor::new(p, policy(), start()).unwrap();
    let noon = at(hours(3.0)); // Mon 12:00 Paris
    g.tick(noon);
    let evening = at(hours(10.0)); // Mon 19:00 Paris
    assert_eq!(g.set_off_hours(&a1, true, noon), Ok(evening));
    let d = g.request_turn(task("A-1", "A", Tier::High), noon);
    assert_eq!(queue_reason(&d), Some(QueueReason::ScheduledOffHours { starts_at: evening }));
    assert_eq!(g.queue()[0].estimated_start, Some(evening));
    assert_eq!(serde_json::to_value(&g.queue()[0].reason).unwrap()["kind"], "scheduled_off_hours");
    // Other work runs now; the scheduled one is released at 19:00.
    assert_eq!(g.request_turn(task("B-1", "B", Tier::Normal), noon).effective, Verdict::Admit);
    assert!(g.tick(at(hours(9.9))).is_empty());
    g.turn_ended(&AgentId::from("B-1"), at(hours(9.9)));
    let released = g.tick(evening);
    assert_eq!(released.iter().map(|r| &r.agent).collect::<Vec<_>>(), [&a1]);
    assert_eq!(g.off_hours_running(), 1);
    // A second scheduled agent waits for the off-hours cap (1).
    let a2 = AgentId::from("B-2");
    g.set_off_hours(&a2, true, evening).unwrap();
    let d = g.request_turn(task("B-2", "B", Tier::Normal), evening);
    assert!(matches!(queue_reason(&d), Some(QueueReason::LiveCap { cap: 1, .. })), "{d:?}");
    // The scheduled turn ends: its mark goes, the next one runs.
    let released = g.turn_ended(&a1, at(hours(11.0)));
    assert_eq!(released.iter().map(|r| &r.agent).collect::<Vec<_>>(), [&a2]);
    assert!(!g.off_hours_agents().contains(&a1));
    // Kept across a restart.
    let c = g.carry();
    assert_eq!(c.off_hours, vec![a2]);
}
