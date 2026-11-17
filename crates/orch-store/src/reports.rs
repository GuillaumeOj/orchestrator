//! P3: weekly reports (FR-818) and the estimator carried across restarts
//! and weeks (FR-814). Both are kept: retention never purges them.

use anyhow::Result;
use governor::jiff::Timestamp;
use governor::{Calibration, EstimatorState};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::db::Store;
use crate::util::{js, json_row_lenient, ms};

/// One stored report's line in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportRow {
    pub id: String,
    pub week_start_ms: i64,
    pub reset_at_ms: i64,
    pub final_weekly_pct: Option<f64>,
}

impl Store {
    /// A calibration with the estimator's state: `week` = the estimator a
    /// week started with (one per week, replaced), `fit` = after a fit change.
    pub fn save_estimator(
        &self,
        kind: &str,
        week_start: Timestamp,
        c: &Calibration,
        state: &EstimatorState,
    ) -> Result<()> {
        let (kind, c, state) = (kind.to_string(), c.clone(), js(state));
        self.write(move |tx| {
            if kind == "week" {
                tx.execute("DELETE FROM calibrations WHERE kind='week' AND week_start_ms=?1", [ms(week_start)])?;
            }
            tx.execute(
                "INSERT INTO calibrations(ts_ms, cost_per_weekly_pct, confidence, unattributed_rate, data, kind, week_start_ms, state)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    ms(c.ts),
                    c.cost_per_weekly_pct,
                    c.confidence,
                    c.unattributed_rate,
                    js(&c),
                    kind,
                    ms(week_start),
                    state
                ],
            )?;
            Ok(())
        })
    }

    /// The estimator the week starting at `week_start` started with.
    pub fn week_estimator(&self, week_start: Timestamp) -> Result<Option<EstimatorState>> {
        self.read(move |c| {
            json_row_lenient(
                c,
                "SELECT state FROM calibrations WHERE kind='week' AND week_start_ms=?1 ORDER BY id DESC LIMIT 1",
                [ms(week_start)],
            )
        })
    }

    /// The latest week's starting estimator before `week_start`: (that week's start, state).
    pub fn week_estimator_before(&self, week_start: Timestamp) -> Result<Option<(Timestamp, EstimatorState)>> {
        self.read(move |c| {
            let row: Option<(i64, String)> = c
                .query_row(
                    "SELECT week_start_ms, state FROM calibrations WHERE kind='week' AND week_start_ms < ?1
                     ORDER BY week_start_ms DESC, id DESC LIMIT 1",
                    [ms(week_start)],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            Ok(row.and_then(|(w, s)| Some((Timestamp::from_millisecond(w).ok()?, serde_json::from_str(&s).ok()?))))
        })
    }

    /// Every week's starting estimator, oldest week first (the latest row of
    /// each week). A row whose state no longer parses is left out.
    pub fn week_estimators(&self) -> Result<Vec<(Timestamp, EstimatorState)>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT week_start_ms, state FROM calibrations c WHERE kind='week'
                   AND id = (SELECT max(id) FROM calibrations WHERE kind='week' AND week_start_ms=c.week_start_ms)
                 ORDER BY week_start_ms",
            )?;
            let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)))?;
            Ok(rows
                .flatten()
                .filter_map(|(w, s)| Some((Timestamp::from_millisecond(w).ok()?, serde_json::from_str(&s?).ok()?)))
                .collect())
        })
    }

    /// Governor actions logged in `[from, to)`, across boots, without the
    /// copies a replay after a restart logs again (same kind, time and reason).
    pub fn actions_between(&self, from: Timestamp, to: Timestamp) -> Result<Vec<governor::GovernorAction>> {
        self.read(move |c| {
            let mut st =
                c.prepare("SELECT data FROM governor_actions WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms, rowid_")?;
            let rows = st.query_map([ms(from), ms(to)], |r| r.get::<_, String>(0))?;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for d in rows.flatten() {
                let Ok(a) = serde_json::from_str::<governor::GovernorAction>(&d) else { continue };
                if seen.insert((a.kind_name(), ms(a.ts), a.reason.clone())) {
                    out.push(a);
                }
            }
            Ok(out)
        })
    }

    /// Stores (or replaces) a finished week's report.
    pub fn save_report(
        &self,
        id: &str,
        week_start: Timestamp,
        reset_at: Timestamp,
        final_pct: Option<f64>,
        data: &Value,
    ) -> Result<()> {
        let (id, data) = (id.to_string(), js(data));
        let now = crate::now_ms() as i64;
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO weekly_reports(id, week_start_ms, reset_at_ms, final_weekly_pct, data, created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(id) DO UPDATE SET week_start_ms=excluded.week_start_ms, reset_at_ms=excluded.reset_at_ms,
                   final_weekly_pct=excluded.final_weekly_pct, data=excluded.data",
                params![id, ms(week_start), ms(reset_at), final_pct, data, now],
            )?;
            Ok(())
        })
    }

    /// Overwrites a stored report's data in place: its id, dates, final % and
    /// creation time stay. False when there is no report `id`.
    pub fn replace_report_data(&self, id: &str, data: &Value) -> Result<bool> {
        let (id, data) = (id.to_string(), js(data));
        self.write(move |tx| Ok(tx.execute("UPDATE weekly_reports SET data=?2 WHERE id=?1", params![id, data])? == 1))
    }

    pub fn report(&self, id: &str) -> Result<Option<Value>> {
        let id = id.to_string();
        self.read(move |c| json_row_lenient(c, "SELECT data FROM weekly_reports WHERE id=?1", [id]))
    }

    /// Stored reports, newest first.
    pub fn reports(&self) -> Result<Vec<ReportRow>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT id, week_start_ms, reset_at_ms, final_weekly_pct FROM weekly_reports ORDER BY reset_at_ms DESC",
            )?;
            let rows = st.query_map([], |r| {
                Ok(ReportRow {
                    id: r.get(0)?,
                    week_start_ms: r.get(1)?,
                    reset_at_ms: r.get(2)?,
                    final_weekly_pct: r.get(3)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// A report already covers the week ending at `reset_at` (± 1 h: a
    /// realigned plan is the same week).
    pub fn report_for_reset(&self, reset_at: Timestamp) -> Result<Option<String>> {
        self.read(move |c| {
            Ok(c.query_row(
                "SELECT id FROM weekly_reports WHERE abs(reset_at_ms - ?1) <= ?2 LIMIT 1",
                [ms(reset_at), governor::SAME_RESET_S * 1000],
                |r| r.get(0),
            )
            .optional()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::{Calibrator, FamilyCalibrator};
    use serde_json::json;

    #[test]
    fn estimators_by_week_and_reports_kept() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("o.db")).unwrap();
        let t = |h: i64| Timestamp::from_second(1_800_000_000 + h * 3600).unwrap();
        let mut cal = Calibrator::new(2.0, 0.9);
        let st = |c: &Calibrator| EstimatorState { calibrator: c.clone(), families: FamilyCalibrator::new(0.9) };
        s.save_estimator("week", t(0), &cal.snapshot(t(0)), &st(&cal)).unwrap();
        cal.update(10.0, 2.0, 4.0);
        s.save_estimator("fit", t(0), &cal.snapshot(t(5)), &st(&cal)).unwrap();
        s.save_estimator("week", t(168), &cal.snapshot(t(168)), &st(&cal)).unwrap();
        s.save_estimator("week", t(168), &cal.snapshot(t(168)), &st(&cal)).unwrap(); // replaced
        assert_eq!(s.week_estimator(t(168)).unwrap().unwrap().calibrator, cal);
        assert_eq!(s.week_estimator(t(0)).unwrap().unwrap().calibrator.intervals(), 0);
        let (w, e) = s.week_estimator_before(t(168)).unwrap().unwrap();
        assert_eq!((w, e.calibrator.intervals()), (t(0), 0));
        assert!(s.week_estimator_before(t(0)).unwrap().is_none());
        let all = s.week_estimators().unwrap();
        assert_eq!(all.iter().map(|(w, _)| *w).collect::<Vec<_>>(), [t(0), t(168)]);
        assert_eq!((all[0].1.calibrator.intervals(), &all[1].1.calibrator), (0, &cal));
        let n: i64 = s
            .read(|c| Ok(c.query_row("SELECT count(*) FROM calibrations WHERE kind='week'", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 2);
        assert!(s.latest_calibration().unwrap().is_some());

        s.save_report("week-1", t(0), t(168), Some(71.0), &json!({"id": "week-1"})).unwrap();
        assert_eq!(s.report("week-1").unwrap().unwrap()["id"], "week-1");
        assert_eq!(s.reports().unwrap()[0].final_weekly_pct, Some(71.0));
        assert_eq!(s.report_for_reset(t(168)).unwrap().as_deref(), Some("week-1"));
        let created = |s: &Store| -> i64 {
            s.read(|c| Ok(c.query_row("SELECT created_at_ms FROM weekly_reports", [], |r| r.get(0))?)).unwrap()
        };
        let before = created(&s);
        assert!(s.replace_report_data("week-1", &json!({"id": "week-1", "v": 2})).unwrap());
        assert!(!s.replace_report_data("week-9", &json!({})).unwrap());
        assert_eq!(s.report("week-1").unwrap().unwrap()["v"], 2);
        let row = &s.reports().unwrap()[0];
        assert_eq!((row.id.as_str(), row.week_start_ms, row.reset_at_ms), ("week-1", ms(t(0)), ms(t(168))));
        assert_eq!((row.final_weekly_pct, created(&s)), (Some(71.0), before));
        assert!(s.report_for_reset(t(170)).unwrap().is_none());
        // Retention leaves reports and calibrations alone.
        s.purge(u64::MAX / 2, crate::Retention::default()).unwrap();
        assert_eq!(s.reports().unwrap().len(), 1);
        assert!(s.week_estimator(t(168)).unwrap().is_some());
    }
}
