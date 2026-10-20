//! Property-based tests: the curve, and invariants of the Governor under random event streams.

mod common;

use std::collections::BTreeSet;

use common::*;
use governor::jiff::Timestamp;
use governor::jiff::civil::time;
use governor::*;
use proptest::prelude::*;

// ---------------------------------------------------------------- curve

fn calendar_strategy() -> impl Strategy<Value = WorkingCalendar> {
    prop::collection::vec((0usize..7, 0i8..12, 1i8..12, 1u8..=4), 1..8).prop_map(|blocks| {
        let days = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri, Day::Sat, Day::Sun];
        let mut used: BTreeSet<usize> = BTreeSet::new();
        let blocks = blocks
            .into_iter()
            .filter(|(d, ..)| used.insert(*d)) // one block per day: no overlaps
            .map(|(d, start, len, w)| WorkBlock {
                day: days[d],
                start: time(start, 0, 0, 0),
                end: time((start + len).min(23), 30, 0, 0),
                weight: w as f64 / 2.0,
            })
            .collect();
        WorkingCalendar { time_zone: "Europe/Paris".into(), blocks, holidays: Vec::new() }
    })
}

proptest! {
    #[test]
    fn curve_is_monotone_and_bounded(
        cal in calendar_strategy(),
        allowance in prop_oneof![Just(0.0), 1.0f64..50.0],
        a in 0i64..10_140, b in 0i64..10_140,
    ) {
        let c = Curve::build(&cal, start(), RESET.parse().unwrap(), allowance).unwrap();
        let (t1, t2) = (at(a.min(b)), at(a.max(b)));
        prop_assert!(c.hours_at(t1) <= c.hours_at(t2) + 1e-9);
        prop_assert!((0.0..=1.0).contains(&c.fraction(t1)));
        prop_assert!((c.fraction(RESET.parse().unwrap()) - 1.0).abs() < 1e-9);
        // The inverse never lands after the original instant.
        if let Some(t) = c.time_at_hours(c.hours_at(t2)) {
            prop_assert!(t <= t2);
            prop_assert!((c.hours_at(t) - c.hours_at(t2)).abs() < 1e-3);
        }
        prop_assert!(c.active_hours_between(t1, t2) <= (t2.as_second() - t1.as_second()) as f64 / 3600.0 + 1e-9);
    }

    #[test]
    fn sprint_keeps_the_total_and_the_past(
        cal in calendar_strategy(),
        from in 0i64..9_000, len in 30i64..900, boost in 0.0f64..3.0,
    ) {
        let c = Curve::build(&cal, start(), RESET.parse().unwrap(), 0.0).unwrap();
        let (f, t) = (at(from), at(from + len));
        let (s, applied) = c.with_sprint(f, t, boost);
        prop_assert!(applied <= boost + 1e-12);
        prop_assert!((s.total_hours() - c.total_hours()).abs() < 1e-6);
        prop_assert!((s.hours_at(f) - c.hours_at(f)).abs() < 1e-9);
        prop_assert!(s.hours_at(t) >= c.hours_at(t) - 1e-9);
        for m in (0..10_140).step_by(240) {
            prop_assert!(s.hours_at(at(m)) <= s.hours_at(at(m + 240)) + 1e-9);
        }
    }
}

// ---------------------------------------------------------------- governor

#[derive(Clone, Debug)]
enum Ev {
    Request { agent: u8, tier: Tier, interactive: bool, self_started: bool },
    Tool { agent: u8, interactive: bool },
    End { agent: u8 },
    Sample { agent: u8, cost: f64, weekly: f64, five: f64 },
    Limit { agent: u8 },
    Pause,
    Resume,
    Tick,
}

fn tier() -> impl Strategy<Value = Tier> {
    prop_oneof![Just(Tier::Low), Just(Tier::Normal), Just(Tier::High), Just(Tier::Critical)]
}

fn ev() -> impl Strategy<Value = Ev> {
    prop_oneof![
        4 => (0u8..8, tier(), prop::bool::weighted(0.1), prop::bool::weighted(0.2))
            .prop_map(|(agent, tier, interactive, self_started)| Ev::Request { agent, tier, interactive, self_started }),
        4 => (0u8..8, prop::bool::weighted(0.1)).prop_map(|(agent, interactive)| Ev::Tool { agent, interactive }),
        3 => (0u8..8).prop_map(|agent| Ev::End { agent }),
        4 => (0u8..8, 0.0f64..40.0, 0.0f64..3.0, 0.0f64..12.0)
            .prop_map(|(agent, cost, weekly, five)| Ev::Sample { agent, cost, weekly, five }),
        1 => (0u8..8).prop_map(|agent| Ev::Limit { agent }),
        1 => Just(Ev::Pause),
        1 => Just(Ev::Resume),
        3 => Just(Ev::Tick),
    ]
}

fn agent_name(i: u8) -> String {
    format!("{}-{i}", ["A", "B", "C"][i as usize % 3])
}

/// Replay events; returns the action log. Checks invariants along the way.
fn replay(events: &[(i64, Ev)], shadow: bool) -> Result<Vec<GovernorAction>, TestCaseError> {
    let mut p = plan();
    p.shadow_mode = shadow;
    let mut g = Governor::new(p, policy(), start()).unwrap();
    let (mut minute, mut w, mut f) = (0i64, 0.0f64, 0.0f64);
    let mut five_reset = at(300);
    for (dt, e) in events {
        minute += dt;
        let now = at(minute);
        if now >= five_reset {
            five_reset = at(minute + 300);
            f = 0.0;
        }
        match e {
            Ev::Request { agent, tier, interactive, self_started } => {
                let name = agent_name(*agent);
                let mut r = TurnRequest::task(name.as_str(), &name[..1], *tier);
                r.interactive = *interactive;
                r.self_started = *self_started;
                let d = g.request_turn(r, now);
                if *interactive || shadow {
                    prop_assert_eq!(&d.effective, &Verdict::Admit);
                }
                if d.effective.is_admit() && !*interactive && *tier != Tier::Critical && !shadow {
                    let cap = g.snapshot(now).live_cap;
                    prop_assert!(
                        g.live_autonomous() <= cap,
                        "admitted over the cap: {} > {}",
                        g.live_autonomous(),
                        cap
                    );
                }
            }
            Ev::Tool { agent, interactive } => {
                let d =
                    g.tool_call(&ToolCallRequest { agent: agent_name(*agent).into(), interactive: *interactive }, now);
                if *interactive || shadow {
                    prop_assert_eq!(&d.effective, &Verdict::Admit);
                }
            }
            Ev::End { agent } => {
                let critical: BTreeSet<EntryId> =
                    g.queue().iter().filter(|e| e.tier == Tier::Critical).map(|e| e.id).collect();
                let released = g.turn_ended(&agent_name(*agent).into(), now);
                // Releases fill free slots only (critical work aside).
                if released.iter().any(|r| !critical.contains(&r.entry)) {
                    let cap = g.snapshot(now).live_cap;
                    prop_assert!(
                        g.live_autonomous() <= cap,
                        "released over the cap: {} > {}",
                        g.live_autonomous(),
                        cap
                    );
                }
            }
            Ev::Sample { agent, cost, weekly, five } => {
                w = (w + weekly).min(99.0);
                f = (f + five).min(99.0);
                g.ingest(&UsageSample {
                    ts: now,
                    agent: Some(agent_name(*agent).into()),
                    cost_delta: *cost,
                    five_hour: Some(WindowReading { used_pct: f.floor(), resets_at: five_reset }),
                    weekly: Some(WindowReading { used_pct: w.floor(), resets_at: RESET.parse().unwrap() }),
                    provenance: Provenance::Observed,
                });
            }
            Ev::Limit { agent } => {
                g.rate_limit_hit(Some(&agent_name(*agent).into()), Window::FiveHour, five_reset, now);
            }
            Ev::Pause => {
                g.pause_all(now);
            }
            Ev::Resume => {
                g.resume_all(now);
            }
            Ev::Tick => {
                g.tick(now);
            }
        }
        // One queue entry per agent; nothing queued in shadow mode.
        let agents: BTreeSet<&AgentId> = g.queue().iter().map(|e| &e.agent).collect();
        prop_assert_eq!(agents.len(), g.queue().len());
        if shadow {
            prop_assert!(g.queue().is_empty());
        }
    }
    // Dwell between state changes.
    let changes: Vec<Timestamp> =
        g.actions().iter().filter(|a| matches!(a.action, ActionKind::StateChange { .. })).map(|a| a.ts).collect();
    let mut prev = start();
    for t in changes {
        prop_assert!(
            t.as_second() - prev.as_second() >= 30 * 60,
            "state changed after {} s",
            t.as_second() - prev.as_second()
        );
        prev = t;
    }
    Ok(g.actions().to_vec())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn invariants_hold_under_random_events(events in prop::collection::vec((1i64..30, ev()), 1..300)) {
        replay(&events, false)?;
    }

    #[test]
    fn shadow_mode_never_enforces(events in prop::collection::vec((1i64..30, ev()), 1..300)) {
        replay(&events, true)?;
    }

    #[test]
    fn decisions_are_deterministic(events in prop::collection::vec((1i64..30, ev()), 1..200)) {
        let a = replay(&events, false)?;
        let b = replay(&events, false)?;
        prop_assert_eq!(a, b);
    }
}
