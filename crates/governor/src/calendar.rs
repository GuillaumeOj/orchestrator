//! Working calendar and the weighted-hours curve `H(t)` behind `ideal(t)` (FR-803).

use jiff::Timestamp;
use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

use crate::types::hours_between;

/// Day of the week, serialisable without depending on jiff's serde coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    pub const WEEKDAYS: [Day; 5] = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri];

    fn matches(self, w: Weekday) -> bool {
        let d = match w {
            Weekday::Monday => Day::Mon,
            Weekday::Tuesday => Day::Tue,
            Weekday::Wednesday => Day::Wed,
            Weekday::Thursday => Day::Thu,
            Weekday::Friday => Day::Fri,
            Weekday::Saturday => Day::Sat,
            Weekday::Sunday => Day::Sun,
        };
        d == self
    }
}

/// A block of working time on one day, in the calendar's time zone.
/// Blocks cannot cross midnight (split them).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkBlock {
    pub day: Day,
    pub start: Time,
    pub end: Time,
    /// How much an hour of this block counts (1.0 = a normal working hour).
    #[serde(default = "one")]
    pub weight: f64,
}

fn one() -> f64 {
    1.0
}

/// Working calendar of a weekly plan (FR-802): days, hours, time zone, holidays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkingCalendar {
    /// IANA time zone name, e.g. `Europe/Paris`.
    pub time_zone: String,
    pub blocks: Vec<WorkBlock>,
    /// Whole days that count as off-hours.
    #[serde(default)]
    pub holidays: Vec<Date>,
}

impl WorkingCalendar {
    /// Same hours on Monday to Friday, weight 1.
    pub fn weekdays(time_zone: &str, start: Time, end: Time) -> Self {
        WorkingCalendar {
            time_zone: time_zone.to_owned(),
            blocks: Day::WEEKDAYS.iter().map(|&day| WorkBlock { day, start, end, weight: 1.0 }).collect(),
            holidays: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarError {
    UnknownTimeZone(String),
    BadBlock(String),
    Overlap(String),
    NoWorkingHours,
    BadWeek,
    BadAllowance,
}

impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarError::UnknownTimeZone(tz) => write!(f, "unknown time zone {tz}"),
            CalendarError::BadBlock(b) => write!(f, "invalid work block: {b}"),
            CalendarError::Overlap(b) => write!(f, "overlapping work blocks: {b}"),
            CalendarError::NoWorkingHours => write!(f, "the week has no weighted hours"),
            CalendarError::BadWeek => write!(f, "week start must be before the reset"),
            CalendarError::BadAllowance => write!(f, "off-hours allowance must be in [0, 100)"),
        }
    }
}

impl std::error::Error for CalendarError {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    start: i64,
    end: i64,
    /// Weighted hours per wall hour.
    weight: f64,
    /// `H` at `start`.
    cum: f64,
}

/// The compiled curve: `H(t)` = weighted hours elapsed since the week start.
///
/// Working blocks count their weight; off-hours count `w_off` per hour, chosen
/// so that off-hours hold exactly `off_hours_allowance` of the total (0 when no
/// allowance is enabled). `fraction(t) = H(t) / H_week`.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    segments: Vec<Segment>,
    total: f64,
    start: i64,
    end: i64,
}

impl Curve {
    /// Build the curve for `[week_start, reset_at)`.
    /// `off_hours_allowance_pct` is the share of the week's budget given to off-hours.
    pub fn build(
        calendar: &WorkingCalendar,
        week_start: Timestamp,
        reset_at: Timestamp,
        off_hours_allowance_pct: f64,
    ) -> Result<Curve, CalendarError> {
        if week_start >= reset_at {
            return Err(CalendarError::BadWeek);
        }
        if !(0.0..100.0).contains(&off_hours_allowance_pct) {
            return Err(CalendarError::BadAllowance);
        }
        let tz = TimeZone::get(&calendar.time_zone)
            .map_err(|_| CalendarError::UnknownTimeZone(calendar.time_zone.clone()))?;
        for b in &calendar.blocks {
            if b.end <= b.start || !b.weight.is_finite() || b.weight < 0.0 {
                return Err(CalendarError::BadBlock(format!("{b:?}")));
            }
        }
        let (ws, we) = (week_start.as_second(), reset_at.as_second());

        // Working intervals, clipped to the week.
        let mut work: Vec<(i64, i64, f64)> = Vec::new();
        let first = week_start.to_zoned(tz.clone()).date().yesterday().map_err(|_| CalendarError::BadWeek)?;
        let last = reset_at.to_zoned(tz.clone()).date().tomorrow().map_err(|_| CalendarError::BadWeek)?;
        let mut date = first;
        while date <= last {
            if !calendar.holidays.contains(&date) {
                for b in calendar.blocks.iter().filter(|b| b.day.matches(date.weekday())) {
                    let s = local_ts(&tz, date, b.start)?;
                    let e = local_ts(&tz, date, b.end)?;
                    let (s, e) = (s.max(ws), e.min(we));
                    if s < e {
                        work.push((s, e, b.weight));
                    }
                }
            }
            date = date.tomorrow().map_err(|_| CalendarError::BadWeek)?;
        }
        work.sort_by_key(|w| (w.0, w.1));
        for pair in work.windows(2) {
            if pair[1].0 < pair[0].1 {
                return Err(CalendarError::Overlap(format!("{pair:?}")));
            }
        }

        // Fill the gaps with off-hours segments (weight decided below).
        let mut raw: Vec<(i64, i64, Option<f64>)> = Vec::new();
        let mut cursor = ws;
        for (s, e, w) in work {
            if s > cursor {
                raw.push((cursor, s, None));
            }
            raw.push((s, e, Some(w)));
            cursor = e;
        }
        if cursor < we {
            raw.push((cursor, we, None));
        }
        let h_work: f64 = raw.iter().filter_map(|&(s, e, w)| w.map(|w| w * (e - s) as f64 / 3600.0)).sum();
        let off_hours: f64 = raw.iter().filter(|r| r.2.is_none()).map(|&(s, e, _)| (e - s) as f64 / 3600.0).sum();
        let a = off_hours_allowance_pct / 100.0;
        let w_off = if a > 0.0 && off_hours > 0.0 {
            if h_work > 0.0 { a / (1.0 - a) * h_work / off_hours } else { 1.0 }
        } else {
            0.0
        };

        let mut segments = Vec::with_capacity(raw.len());
        let mut cum = 0.0;
        for (s, e, w) in raw {
            let weight = w.unwrap_or(w_off);
            segments.push(Segment { start: s, end: e, weight, cum });
            cum += weight * (e - s) as f64 / 3600.0;
        }
        if cum <= 0.0 {
            return Err(CalendarError::NoWorkingHours);
        }
        Ok(Curve { segments, total: cum, start: ws, end: we })
    }

    pub fn week_start(&self) -> Timestamp {
        Timestamp::from_second(self.start).expect("valid")
    }

    pub fn reset_at(&self) -> Timestamp {
        Timestamp::from_second(self.end).expect("valid")
    }

    /// `H_week`.
    pub fn total_hours(&self) -> f64 {
        self.total
    }

    /// `H(t)`, clamped to the week.
    pub fn hours_at(&self, t: Timestamp) -> f64 {
        let s = t.as_second();
        if s <= self.start {
            return 0.0;
        }
        if s >= self.end {
            return self.total;
        }
        let i = self.segments.partition_point(|g| g.end <= s);
        let g = &self.segments[i.min(self.segments.len() - 1)];
        g.cum + g.weight * (s - g.start) as f64 / 3600.0
    }

    /// `H(t) / H_week` in `[0, 1]`.
    pub fn fraction(&self, t: Timestamp) -> f64 {
        (self.hours_at(t) / self.total).clamp(0.0, 1.0)
    }

    /// `H_week − H(t)`.
    pub fn hours_left(&self, t: Timestamp) -> f64 {
        (self.total - self.hours_at(t)).max(0.0)
    }

    /// Weight (weighted hours per wall hour) in force at `t`; 0 outside the week.
    pub fn weight_at(&self, t: Timestamp) -> f64 {
        let s = t.as_second();
        if s < self.start || s >= self.end {
            return 0.0;
        }
        let i = self.segments.partition_point(|g| g.end <= s);
        self.segments.get(i).map_or(0.0, |g| g.weight)
    }

    /// The earliest instant `>= from` whose weight is positive (next working time).
    pub fn next_active(&self, from: Timestamp) -> Option<Timestamp> {
        let s = from.as_second();
        self.segments
            .iter()
            .filter(|g| g.end > s && g.weight > 0.0)
            .map(|g| g.start.max(s))
            .next()
            .and_then(|x| Timestamp::from_second(x).ok())
    }

    /// The earliest instant at which `H` reaches `h`, or `None` if it never does in the week.
    pub fn time_at_hours(&self, h: f64) -> Option<Timestamp> {
        if h <= 0.0 {
            return Some(self.week_start());
        }
        if h > self.total + 1e-9 {
            return None;
        }
        for g in &self.segments {
            let seg_h = g.weight * (g.end - g.start) as f64 / 3600.0;
            if g.weight > 0.0 && g.cum + seg_h >= h {
                let secs = ((h - g.cum) / g.weight * 3600.0 - 1e-6).ceil().max(0.0) as i64;
                return Timestamp::from_second((g.start + secs).min(g.end)).ok();
            }
        }
        Some(self.reset_at())
    }

    /// Active wall-clock hours (weight > 0, when autonomous work runs) in `[from, to)`.
    pub fn active_hours_between(&self, from: Timestamp, to: Timestamp) -> f64 {
        let (f, t) = (from.as_second(), to.as_second());
        self.segments
            .iter()
            .filter(|g| g.weight > 0.0)
            .map(|g| (g.end.min(t) - g.start.max(f)).max(0) as f64 / 3600.0)
            .sum()
    }

    /// The instant after `hours` of active wall-clock time from `from`, or
    /// `None` if the week ends first.
    pub fn after_active_hours(&self, from: Timestamp, hours: f64) -> Option<Timestamp> {
        let f = from.as_second();
        let mut left = hours.max(0.0) * 3600.0;
        for g in self.segments.iter().filter(|g| g.weight > 0.0 && g.end > f) {
            let start = g.start.max(f);
            let len = (g.end - start) as f64;
            if left <= len {
                return Timestamp::from_second(start + left.ceil() as i64).ok();
            }
            left -= len;
        }
        None
    }

    /// Wall-clock hours between `t` and the reset.
    pub fn wall_hours_left(&self, t: Timestamp) -> f64 {
        hours_between(t, self.reset_at()).max(0.0)
    }

    /// Sprint mode as borrowing (FR-811): weights in `[from, to)` are multiplied
    /// by `1 + boost` and every weight after `to` is scaled down so that `H_week`
    /// is unchanged. `H(t)` before `from` is unchanged, so the curve stays
    /// continuous. The boost is clamped so later weights stay `>= 0`.
    /// Returns the new curve and the boost actually applied.
    pub fn with_sprint(&self, from: Timestamp, to: Timestamp, boost: f64) -> (Curve, f64) {
        let (f, t) = (from.as_second().max(self.start), to.as_second().min(self.end));
        if f >= t || boost <= 0.0 {
            return (self.clone(), 0.0);
        }
        let split = self.split_at(f).split_at(t);
        let h_sprint: f64 = split.segment_hours(|g| g.start >= f && g.end <= t);
        let h_after: f64 = split.segment_hours(|g| g.start >= t);
        if h_sprint <= 0.0 {
            return (self.clone(), 0.0);
        }
        let boost = if h_after > 0.0 { boost.min(h_after / h_sprint) } else { 0.0 };
        let scale_after = if h_after > 0.0 { (h_after - boost * h_sprint) / h_after } else { 1.0 };
        let mut segments = Vec::with_capacity(split.segments.len());
        let mut cum = 0.0;
        for g in &split.segments {
            let weight = if g.start >= f && g.end <= t {
                g.weight * (1.0 + boost)
            } else if g.start >= t {
                g.weight * scale_after
            } else {
                g.weight
            };
            segments.push(Segment { start: g.start, end: g.end, weight, cum });
            cum += weight * (g.end - g.start) as f64 / 3600.0;
        }
        (Curve { segments, total: cum, start: self.start, end: self.end }, boost)
    }

    fn segment_hours(&self, pred: impl Fn(&Segment) -> bool) -> f64 {
        self.segments.iter().filter(|g| pred(g)).map(|g| g.weight * (g.end - g.start) as f64 / 3600.0).sum()
    }

    fn split_at(&self, s: i64) -> Curve {
        let mut segments = Vec::with_capacity(self.segments.len() + 1);
        for g in &self.segments {
            if g.start < s && s < g.end {
                segments.push(Segment { end: s, ..*g });
                let cum = g.cum + g.weight * (s - g.start) as f64 / 3600.0;
                segments.push(Segment { start: s, cum, ..*g });
            } else {
                segments.push(*g);
            }
        }
        Curve { segments, ..self.clone() }
    }

    /// End of the last working block (positive weight) that starts on the same
    /// local day as `t`, used as the default end of a sprint.
    pub fn end_of_working_day(&self, t: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        let day = t.to_zoned(tz.clone()).date();
        let s = t.as_second();
        self.segments
            .iter()
            .filter(|g| g.weight > 0.0 && g.end > s)
            .filter(|g| {
                Timestamp::from_second(g.start).map(|ts| ts.to_zoned(tz.clone()).date() == day).unwrap_or(false)
            })
            .map(|g| g.end)
            .max()
            .and_then(|e| Timestamp::from_second(e).ok())
    }
}

fn local_ts(tz: &TimeZone, date: Date, time: Time) -> Result<i64, CalendarError> {
    date.to_datetime(time)
        .to_zoned(tz.clone())
        .map(|z| z.timestamp().as_second())
        .map_err(|e| CalendarError::BadBlock(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::time;

    fn paris_week() -> (WorkingCalendar, Timestamp, Timestamp) {
        let cal = WorkingCalendar::weekdays("Europe/Paris", time(9, 0, 0, 0), time(19, 0, 0, 0));
        // Monday 2026-10-19 09:00 Paris (UTC+2) to the next Monday.
        let start: Timestamp = "2026-10-19T07:00:00Z".parse().unwrap();
        let reset: Timestamp = "2026-10-26T08:00:00Z".parse().unwrap(); // DST ended on Oct 25
        (cal, start, reset)
    }

    #[test]
    fn weekday_hours_and_fraction() {
        let (cal, start, reset) = paris_week();
        let c = Curve::build(&cal, start, reset, 0.0).unwrap();
        assert!((c.total_hours() - 50.0).abs() < 1e-9);
        let noon_mon: Timestamp = "2026-10-19T10:00:00Z".parse().unwrap();
        assert!((c.hours_at(noon_mon) - 3.0).abs() < 1e-9);
        // Monday night counts nothing.
        let mon_23: Timestamp = "2026-10-19T21:00:00Z".parse().unwrap();
        assert!((c.hours_at(mon_23) - 10.0).abs() < 1e-9);
        assert_eq!(c.weight_at(mon_23), 0.0);
        assert!((c.fraction(reset) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn dst_change_is_handled_in_local_time() {
        let (cal, start, reset) = paris_week();
        let c = Curve::build(&cal, start, reset, 0.0).unwrap();
        // Friday 2026-10-23 is still UTC+2; the next Monday 09:00 is UTC+1 = 08:00Z.
        let fri_end: Timestamp = "2026-10-23T17:00:00Z".parse().unwrap();
        assert!((c.hours_at(fri_end) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn holidays_and_weights() {
        let (mut cal, start, reset) = paris_week();
        cal.holidays.push("2026-10-21".parse().unwrap());
        cal.blocks.retain(|b| b.day != Day::Fri);
        cal.blocks.push(WorkBlock { day: Day::Fri, start: time(9, 0, 0, 0), end: time(13, 0, 0, 0), weight: 0.5 });
        let c = Curve::build(&cal, start, reset, 0.0).unwrap();
        assert!((c.total_hours() - (30.0 + 2.0)).abs() < 1e-9);
    }

    #[test]
    fn off_hours_allowance_takes_its_share() {
        let (cal, start, reset) = paris_week();
        let c = Curve::build(&cal, start, reset, 20.0).unwrap();
        let mon_19: Timestamp = "2026-10-19T17:00:00Z".parse().unwrap();
        let work_share = 10.0 / c.total_hours();
        assert!((c.total_hours() - 62.5).abs() < 1e-9, "{}", c.total_hours());
        assert!(c.weight_at("2026-10-19T21:00:00Z".parse().unwrap()) > 0.0);
        assert!((c.fraction(mon_19) - work_share).abs() < 1e-9);
    }

    #[test]
    fn inverse_and_next_active() {
        let (cal, start, reset) = paris_week();
        let c = Curve::build(&cal, start, reset, 0.0).unwrap();
        let t = c.time_at_hours(12.0).unwrap();
        assert_eq!(t, "2026-10-20T09:00:00Z".parse::<Timestamp>().unwrap());
        let night: Timestamp = "2026-10-19T20:00:00Z".parse().unwrap();
        assert_eq!(c.next_active(night).unwrap(), "2026-10-20T07:00:00Z".parse::<Timestamp>().unwrap());
        assert!(c.time_at_hours(51.0).is_none());
    }

    #[test]
    fn active_hours_skip_off_hours() {
        let (cal, start, reset) = paris_week();
        let c = Curve::build(&cal, start, reset, 0.0).unwrap();
        let mon_18: Timestamp = "2026-10-19T16:00:00Z".parse().unwrap();
        let tue_10: Timestamp = "2026-10-20T08:00:00Z".parse().unwrap();
        assert!((c.active_hours_between(mon_18, tue_10) - 2.0).abs() < 1e-9);
        assert_eq!(c.after_active_hours(mon_18, 2.0).unwrap(), tue_10);
        assert!(c.after_active_hours(mon_18, 100.0).is_none());
    }

    #[test]
    fn sprint_borrows_without_changing_the_total() {
        let (cal, start, reset) = paris_week();
        let c = Curve::build(&cal, start, reset, 0.0).unwrap();
        let from: Timestamp = "2026-10-21T07:00:00Z".parse().unwrap();
        let to: Timestamp = "2026-10-21T17:00:00Z".parse().unwrap();
        let (s, boost) = c.with_sprint(from, to, 0.5);
        assert_eq!(boost, 0.5);
        assert!((s.total_hours() - c.total_hours()).abs() < 1e-9);
        assert_eq!(s.hours_at(from), c.hours_at(from));
        assert!((s.hours_at(to) - (c.hours_at(to) + 5.0)).abs() < 1e-9);
        let thu: Timestamp = "2026-10-22T17:00:00Z".parse().unwrap();
        assert!(s.hours_at(thu) > c.hours_at(thu));
        assert!(s.hours_at(thu) < c.hours_at(thu) + 5.0);
        // Borrowing more than remains is clamped.
        let (_, b) = c.with_sprint(from, to, 100.0);
        assert!((b - 2.0).abs() < 1e-9, "{b}");
    }

    #[test]
    fn unknown_zone_is_an_error() {
        let (mut cal, start, reset) = paris_week();
        cal.time_zone = "Mars/Olympus".into();
        assert!(matches!(Curve::build(&cal, start, reset, 0.0), Err(CalendarError::UnknownTimeZone(_))));
    }
}
