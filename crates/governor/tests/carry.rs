//! FR-814: the calibration carries across a restart and a weekly reset. A
//! restart restores the estimator its week started with and replays the
//! week's samples; across a reset it first replays the previous week from
//! that week's own start. Either way `k`, `u` and the confidence are the live
//! run's, with per-model weights off and on.

mod common;

use common::*;
use governor::jiff::{SignedDuration, Timestamp};
use governor::*;

fn week2() -> WeeklyPlan {
    WeeklyPlan { id: "w2".into(), week_start: reset(), reset_at: reset2(), ..plan() }
}

fn reset2() -> Timestamp {
    reset().checked_add(SignedDuration::from_hours(168)).unwrap()
}

fn after(t: Timestamp, minutes: i64) -> Timestamp {
    t.checked_add(SignedDuration::from_mins(minutes)).unwrap()
}

/// Truth: opus 40 cost/pp, sonnet 8 cost/pp, 0.1 pp/h outside the app.
/// A sample every 30 min with an opus or sonnet cost, and the weekly reading.
fn samples(from: Timestamp, resets_at: Timestamp, hours: i64, seed: u64) -> Vec<UsageSample> {
    let mut out = Vec::new();
    let mut w = 0.0;
    let mut x = seed;
    for i in 0..hours * 2 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let opus = (x >> 33) % 3 == 0;
        let cost = 4.0 + ((x >> 40) % 16) as f64;
        w += if opus { cost / 40.0 } else { cost / 8.0 } + 0.05;
        let ts = after(from, 30 * i + 15);
        out.push(UsageSample {
            ts,
            agent: Some("A-1".into()),
            cost_delta: cost,
            five_hour: None,
            weekly: Some(WindowReading { used_pct: w.floor().min(99.0), resets_at }),
            provenance: Provenance::Observed,
            model: Some(if opus { "claude-opus-4-1" } else { "claude-sonnet-4-5" }.into()),
            model_windows: Vec::new(),
        });
    }
    out
}

/// Equal up to rounding: the live run's cumulative costs start from last
/// week's total, a replay's from 0, so sums differ in the last bits.
fn same(a: &Calibration, b: &Calibration, what: &str) {
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0);
    assert!(
        a.ts == b.ts
            && close(a.cost_per_weekly_pct, b.cost_per_weekly_pct)
            && close(a.confidence, b.confidence)
            && close(a.unattributed_rate, b.unattributed_rate),
        "{what}:\n{a:?}\n{b:?}"
    );
    match (&a.per_model, &b.per_model) {
        (None, None) => {}
        (Some(x), Some(y)) => {
            assert_eq!(x.keys().collect::<Vec<_>>(), y.keys().collect::<Vec<_>>(), "{what}");
            for (f, m) in x {
                let n = y[f];
                assert!(close(m.k, n.k) && m.samples == n.samples && m.fitted == n.fitted, "{what} {f}: {m:?} {n:?}");
            }
        }
        _ => panic!("{what}: per_model {:?} vs {:?}", a.per_model, b.per_model),
    }
}

fn policy_with(per_model: bool) -> PacingPolicy {
    PacingPolicy { per_model_weights: per_model, ..policy() }
}

fn check(per_model: bool) {
    let week1 = samples(start(), reset(), 160, 7);
    let week2_all = samples(reset(), reset2(), 60, 11);
    let (w2_done, _) = week2_all.split_at(80); // 40 h into week 2
    let now = after(reset(), 40 * 60);

    // Live: week 1, the reset (new_week at the first reading of week 2), week 2.
    let mut live = Governor::new(plan(), policy_with(per_model), start()).unwrap();
    let base1 = live.week_estimator().clone();
    for s in &week1 {
        live.ingest(s);
    }
    let mut base2 = None;
    for s in w2_done {
        if live.plan().id != "w2" {
            live.new_week(week2(), s.ts).unwrap();
            base2 = Some(live.week_estimator().clone());
        }
        live.ingest(s);
    }
    let base2 = base2.unwrap();
    let k_live = live.calibration(now);
    assert!(k_live.confidence > 0.5, "the fit learned something: {k_live:?}");

    // Restart mid-week 2: the stored base of week 2, the week's samples replayed.
    let replay = |g: &mut Governor, ss: &[UsageSample]| {
        for s in ss {
            g.replay(s, CostClass::Autonomous, Some("A".into()));
        }
    };
    let mut restarted = Governor::new(week2(), policy_with(per_model), reset()).unwrap();
    restarted.restore_estimator(base2.clone());
    replay(&mut restarted, w2_done);
    same(&restarted.calibration(now), &k_live, "restart mid-week");

    // Restart after the reset with no base for week 2: rebuilt from week 1's.
    let mut prev = Governor::new(plan(), policy_with(per_model), start()).unwrap();
    prev.restore_estimator(base1);
    replay(&mut prev, &week1);
    let mut across = Governor::new(week2(), policy_with(per_model), reset()).unwrap();
    across.restore_estimator(prev.estimator());
    same(
        &across.week_estimator().calibrator.snapshot(now),
        &base2.calibrator.snapshot(now),
        "the rebuilt base is the live one",
    );
    replay(&mut across, w2_done);
    same(&across.calibration(now), &k_live, "restart across the reset");

    // Without the carry-over (P2), a restart lost the previous weeks' fit.
    let mut cold = Governor::new(week2(), policy_with(per_model), reset()).unwrap();
    replay(&mut cold, w2_done);
    assert_ne!(cold.calibration(now).cost_per_weekly_pct, k_live.cost_per_weekly_pct);

    if per_model {
        let pm = k_live.per_model.clone().expect("per-model k in the snapshot");
        let (o, s) = (pm["opus"], pm["sonnet"]);
        assert!(o.fitted && s.fitted && o.samples >= MIN_FAMILY_SAMPLES, "{pm:?}");
        assert!(o.k > 2.0 * s.k, "opus costs more per pp: {pm:?}");
        assert_eq!(live.snapshot(now).calibration.per_model, Some(pm));
    } else {
        assert_eq!(k_live.per_model, None);
    }
}

#[test]
fn calibration_carries_across_a_restart_and_a_reset() {
    check(false);
}

#[test]
fn per_model_weights_carry_across_a_restart_and_a_reset() {
    check(true);
}
