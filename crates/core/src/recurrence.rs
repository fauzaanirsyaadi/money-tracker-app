//! Recurring schedules and catch-up.
//!
//! There is no background service: when the app opens, the storage layer asks
//! [`Schedule::catch_up`] which occurrences became due since the last run, and
//! records them. The same inputs always give the same output, so running the
//! catch-up twice never creates a duplicate.
//!
//! Occurrences are computed from the start date by index (`start + k * every`),
//! never by repeatedly adding to the previous date, so "the 31st" does not
//! drift to the 28th after a short month.

use std::fmt;

use time::{Date, Duration, Month, util::days_in_month};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Frequency {
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "yearly" => Some(Self::Yearly),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleError {
    /// `every` must be at least 1.
    InvalidEvery,
    /// `anchor_day` must be 1..=31 and consistent with the start date.
    InvalidAnchorDay,
    /// The stored next-due date is not an occurrence of this schedule.
    OffSchedule,
    /// Text is not a canonical `YYYY-MM-DD` date.
    InvalidDate,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEvery => "interval must be at least 1",
            Self::InvalidAnchorDay => "invalid anchor day",
            Self::OffSchedule => "next due date is not on the schedule",
            Self::InvalidDate => "invalid date",
        })
    }
}

impl std::error::Error for ScheduleError {}

/// Occurrences that became due, and where the schedule continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchUp {
    /// Due dates in ascending order, all `<= today` and `<= end`.
    pub due: Vec<Date>,
    /// First occurrence not yet generated, or `None` once the schedule has
    /// passed its end date (the rule is finished).
    pub next_due: Option<Date>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    frequency: Frequency,
    every: u32,
    anchor_day: u8,
    start: Date,
    end: Option<Date>,
}

impl Schedule {
    /// `anchor_day` is the day of month for monthly/yearly schedules, clamped
    /// to the month length (31 becomes 30 in April, 28/29 in February). The
    /// start date must itself be an occurrence.
    pub fn new(
        frequency: Frequency,
        every: u32,
        anchor_day: u8,
        start: Date,
        end: Option<Date>,
    ) -> Result<Self, ScheduleError> {
        if every == 0 {
            return Err(ScheduleError::InvalidEvery);
        }
        if !(1..=31).contains(&anchor_day) {
            return Err(ScheduleError::InvalidAnchorDay);
        }
        let schedule = Self {
            frequency,
            every,
            anchor_day,
            start,
            end,
        };
        if matches!(frequency, Frequency::Monthly | Frequency::Yearly)
            && schedule.nth(0) != Some(start)
        {
            return Err(ScheduleError::InvalidAnchorDay);
        }
        Ok(schedule)
    }

    /// The `k`-th occurrence (0-based), ignoring `end`. `None` past the
    /// calendar's supported range.
    fn nth(&self, k: u64) -> Option<Date> {
        let step = u64::from(self.every).checked_mul(k)?;
        match self.frequency {
            Frequency::Daily => self
                .start
                .checked_add(Duration::days(i64::try_from(step).ok()?)),
            Frequency::Weekly => {
                let days = step.checked_mul(7)?;
                self.start
                    .checked_add(Duration::days(i64::try_from(days).ok()?))
            }
            Frequency::Monthly => {
                let base =
                    i64::from(self.start.year()) * 12 + i64::from(self.start.month() as u8) - 1;
                let total = base.checked_add(i64::try_from(step).ok()?)?;
                self.clamped(
                    i32::try_from(total.div_euclid(12)).ok()?,
                    total.rem_euclid(12) as u8 + 1,
                )
            }
            Frequency::Yearly => {
                let year = i64::from(self.start.year()).checked_add(i64::try_from(step).ok()?)?;
                self.clamped(i32::try_from(year).ok()?, self.start.month() as u8)
            }
        }
    }

    fn clamped(&self, year: i32, month: u8) -> Option<Date> {
        let month = Month::try_from(month).ok()?;
        let day = self.anchor_day.min(days_in_month(month, year));
        Date::from_calendar_date(year, month, day).ok()
    }

    /// Index `k` such that `nth(k) == due`.
    fn index_of(&self, due: Date) -> Result<u64, ScheduleError> {
        let every = i64::from(self.every);
        let index = match self.frequency {
            Frequency::Daily | Frequency::Weekly => {
                let unit = if self.frequency == Frequency::Weekly {
                    7 * every
                } else {
                    every
                };
                let days = (due - self.start).whole_days();
                if days < 0 || days % unit != 0 {
                    return Err(ScheduleError::OffSchedule);
                }
                days / unit
            }
            Frequency::Monthly | Frequency::Yearly => {
                let months = |d: Date| i64::from(d.year()) * 12 + i64::from(d.month() as u8) - 1;
                let diff = months(due) - months(self.start);
                let unit = if self.frequency == Frequency::Yearly {
                    12 * every
                } else {
                    every
                };
                if diff < 0 || diff % unit != 0 {
                    return Err(ScheduleError::OffSchedule);
                }
                diff / unit
            }
        };
        let index = u64::try_from(index).map_err(|_| ScheduleError::OffSchedule)?;
        // Confirms the day of month too, e.g. the 16th of a schedule on the 15th.
        if self.nth(index) == Some(due) {
            Ok(index)
        } else {
            Err(ScheduleError::OffSchedule)
        }
    }

    /// Generates every occurrence from `next_due` up to and including `today`,
    /// at most `limit` of them. When the limit is hit, `next_due` in the result
    /// is where the next call should resume.
    pub fn catch_up(
        &self,
        next_due: Date,
        today: Date,
        limit: usize,
    ) -> Result<CatchUp, ScheduleError> {
        let mut k = self.index_of(next_due)?;
        let mut due = Vec::new();
        loop {
            let Some(date) = self.nth(k) else {
                return Ok(CatchUp {
                    due,
                    next_due: None,
                });
            };
            if self.end.is_some_and(|end| date > end) {
                return Ok(CatchUp {
                    due,
                    next_due: None,
                });
            }
            if date > today || due.len() >= limit {
                return Ok(CatchUp {
                    due,
                    next_due: Some(date),
                });
            }
            due.push(date);
            k += 1;
        }
    }
}

/// Parses a canonical `YYYY-MM-DD` date (four-digit year, zero-padded).
pub fn parse_iso_date(text: &str) -> Result<Date, ScheduleError> {
    let bytes = text.as_bytes();
    let shape_ok = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shape_ok {
        return Err(ScheduleError::InvalidDate);
    }
    let year: i32 = text[0..4].parse().map_err(|_| ScheduleError::InvalidDate)?;
    let month: u8 = text[5..7].parse().map_err(|_| ScheduleError::InvalidDate)?;
    let day: u8 = text[8..10]
        .parse()
        .map_err(|_| ScheduleError::InvalidDate)?;
    let month = Month::try_from(month).map_err(|_| ScheduleError::InvalidDate)?;
    Date::from_calendar_date(year, month, day).map_err(|_| ScheduleError::InvalidDate)
}

pub fn format_iso_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Date {
        parse_iso_date(text).unwrap()
    }

    fn dates(list: &[Date]) -> Vec<String> {
        list.iter().map(|&x| format_iso_date(x)).collect()
    }

    fn monthly(start: &str, anchor: u8) -> Schedule {
        Schedule::new(Frequency::Monthly, 1, anchor, d(start), None).unwrap()
    }

    // --- dates ----------------------------------------------------------------

    #[test]
    fn iso_dates_round_trip_and_reject_garbage() {
        assert_eq!(format_iso_date(d("2026-10-14")), "2026-10-14");
        assert_eq!(format_iso_date(d("0999-01-01")), "0999-01-01");
        for bad in [
            "",
            "2026-1-1",
            "2026/10/14",
            "2026-13-01",
            "2026-02-30",
            "26-10-14",
            "2026-10-14T0",
            "２０２６-10-14",
            "2026-10-1x",
        ] {
            assert_eq!(
                parse_iso_date(bad),
                Err(ScheduleError::InvalidDate),
                "{bad:?}"
            );
        }
        assert!(parse_iso_date("2024-02-29").is_ok());
        assert!(parse_iso_date("2025-02-29").is_err());
    }

    // --- validation -----------------------------------------------------------

    #[test]
    fn rejects_invalid_schedules() {
        let start = d("2026-01-15");
        assert_eq!(
            Schedule::new(Frequency::Monthly, 0, 15, start, None),
            Err(ScheduleError::InvalidEvery)
        );
        assert_eq!(
            Schedule::new(Frequency::Monthly, 1, 0, start, None),
            Err(ScheduleError::InvalidAnchorDay)
        );
        assert_eq!(
            Schedule::new(Frequency::Monthly, 1, 32, start, None),
            Err(ScheduleError::InvalidAnchorDay)
        );
        // Start on the 15th but anchored to the 20th: start is not an occurrence.
        assert_eq!(
            Schedule::new(Frequency::Monthly, 1, 20, start, None),
            Err(ScheduleError::InvalidAnchorDay)
        );
        // Anchored to the 31st, starting 30 Jan, is also inconsistent...
        assert!(Schedule::new(Frequency::Monthly, 1, 31, d("2026-01-30"), None).is_err());
        // ...but starting on 28 Feb with anchor 31 is a valid clamped start.
        assert!(Schedule::new(Frequency::Monthly, 1, 31, d("2026-02-28"), None).is_ok());
    }

    // --- occurrences ------------------------------------------------------------

    #[test]
    fn month_end_does_not_drift() {
        let schedule = monthly("2026-01-31", 31);
        let result = schedule
            .catch_up(d("2026-01-31"), d("2026-05-31"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            [
                "2026-01-31",
                "2026-02-28",
                "2026-03-31",
                "2026-04-30",
                "2026-05-31"
            ]
        );
        assert_eq!(result.next_due, Some(d("2026-06-30")));
    }

    #[test]
    fn leap_years_are_respected() {
        let monthly_result = monthly("2028-01-31", 31)
            .catch_up(d("2028-01-31"), d("2028-03-01"), 100)
            .unwrap();
        assert_eq!(dates(&monthly_result.due), ["2028-01-31", "2028-02-29"]);

        let yearly = Schedule::new(Frequency::Yearly, 1, 29, d("2024-02-29"), None).unwrap();
        let result = yearly
            .catch_up(d("2024-02-29"), d("2028-12-31"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            [
                "2024-02-29",
                "2025-02-28",
                "2026-02-28",
                "2027-02-28",
                "2028-02-29"
            ]
        );
    }

    #[test]
    fn every_n_applies_to_all_frequencies() {
        let daily = Schedule::new(Frequency::Daily, 3, 1, d("2026-01-01"), None).unwrap();
        let result = daily
            .catch_up(d("2026-01-01"), d("2026-01-10"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            ["2026-01-01", "2026-01-04", "2026-01-07", "2026-01-10"]
        );

        let weekly = Schedule::new(Frequency::Weekly, 2, 1, d("2026-01-05"), None).unwrap();
        let result = weekly
            .catch_up(d("2026-01-05"), d("2026-02-10"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            ["2026-01-05", "2026-01-19", "2026-02-02"]
        );

        let quarterly = Schedule::new(Frequency::Monthly, 3, 10, d("2026-01-10"), None).unwrap();
        let result = quarterly
            .catch_up(d("2026-01-10"), d("2026-12-31"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            ["2026-01-10", "2026-04-10", "2026-07-10", "2026-10-10"]
        );
    }

    // --- catch-up -----------------------------------------------------------------

    #[test]
    fn catches_up_a_gap_and_reports_where_to_continue() {
        let schedule = monthly("2026-01-15", 15);
        let result = schedule
            .catch_up(d("2026-01-15"), d("2026-04-20"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            ["2026-01-15", "2026-02-15", "2026-03-15", "2026-04-15"]
        );
        assert_eq!(result.next_due, Some(d("2026-05-15")));
    }

    #[test]
    fn nothing_is_generated_before_the_due_date() {
        let schedule = monthly("2026-01-15", 15);
        let result = schedule
            .catch_up(d("2026-05-15"), d("2026-05-14"), 100)
            .unwrap();
        assert!(result.due.is_empty());
        assert_eq!(result.next_due, Some(d("2026-05-15")));
    }

    #[test]
    fn running_twice_is_idempotent() {
        let schedule = monthly("2026-01-15", 15);
        let first = schedule
            .catch_up(d("2026-01-15"), d("2026-04-20"), 100)
            .unwrap();
        let second = schedule
            .catch_up(first.next_due.unwrap(), d("2026-04-20"), 100)
            .unwrap();
        assert!(second.due.is_empty());
        assert_eq!(second.next_due, first.next_due);
    }

    #[test]
    fn end_date_finishes_the_rule() {
        let schedule = Schedule::new(
            Frequency::Monthly,
            1,
            15,
            d("2026-01-15"),
            Some(d("2026-03-15")),
        )
        .unwrap();
        let result = schedule
            .catch_up(d("2026-01-15"), d("2026-12-31"), 100)
            .unwrap();
        assert_eq!(
            dates(&result.due),
            ["2026-01-15", "2026-02-15", "2026-03-15"]
        );
        assert_eq!(result.next_due, None);
    }

    #[test]
    fn limit_caps_work_and_resumes_exactly() {
        let schedule = Schedule::new(Frequency::Daily, 1, 1, d("2020-01-01"), None).unwrap();
        let today = d("2026-10-14");
        let first = schedule.catch_up(d("2020-01-01"), today, 1000).unwrap();
        assert_eq!(first.due.len(), 1000);
        assert_eq!(format_iso_date(*first.due.last().unwrap()), "2022-09-26");
        assert_eq!(first.next_due, Some(d("2022-09-27")));

        let second = schedule
            .catch_up(first.next_due.unwrap(), today, 1000)
            .unwrap();
        assert_eq!(second.due.first().copied(), first.next_due);
        // No date is produced twice across the two calls.
        assert!(second.due.iter().all(|x| !first.due.contains(x)));
    }

    #[test]
    fn occurrences_are_strictly_increasing() {
        for (freq, every, anchor, start) in [
            (Frequency::Monthly, 1, 31, "2026-01-31"),
            (Frequency::Monthly, 5, 29, "2026-01-29"),
            (Frequency::Yearly, 2, 29, "2024-02-29"),
            (Frequency::Weekly, 1, 1, "2026-01-01"),
        ] {
            let schedule = Schedule::new(freq, every, anchor, d(start), None).unwrap();
            let result = schedule
                .catch_up(d(start), d("2040-12-31"), 10_000)
                .unwrap();
            assert!(result.due.windows(2).all(|w| w[0] < w[1]), "{freq:?}");
            assert_eq!(result.due.first().copied(), Some(d(start)));
        }
    }

    #[test]
    fn off_schedule_state_is_reported_not_guessed() {
        let schedule = monthly("2026-01-15", 15);
        for bad in ["2026-01-16", "2025-12-15", "2026-02-14"] {
            assert_eq!(
                schedule.catch_up(d(bad), d("2026-12-31"), 10),
                Err(ScheduleError::OffSchedule),
                "{bad}"
            );
        }
        let every_two = Schedule::new(Frequency::Monthly, 2, 15, d("2026-01-15"), None).unwrap();
        assert_eq!(
            every_two.catch_up(d("2026-02-15"), d("2026-12-31"), 10),
            Err(ScheduleError::OffSchedule)
        );
    }

    #[test]
    fn running_off_the_end_of_the_calendar_finishes_the_rule() {
        let schedule = Schedule::new(Frequency::Yearly, 1, 1, d("9998-01-01"), None).unwrap();
        let result = schedule
            .catch_up(d("9998-01-01"), d("9999-12-31"), 10)
            .unwrap();
        assert_eq!(dates(&result.due), ["9998-01-01", "9999-01-01"]);
        assert_eq!(result.next_due, None);
    }
}
