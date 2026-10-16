//! Monthly budgets: which month, and how far along a category is.

use std::fmt;

use time::{Date, Month, util::days_in_month};

use crate::{error::MoneyError, money::Money};

/// Usage at or above this share of the limit (basis points) is a warning.
pub const WARN_AT_BP: i64 = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    /// Not a `YYYY-MM` month.
    InvalidMonth,
    /// A budget limit must be above zero.
    NonPositiveLimit,
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidMonth => "month must be YYYY-MM",
            Self::NonPositiveLimit => "budget limit must be above zero",
        })
    }
}

impl std::error::Error for BudgetError {}

/// A calendar month, written `YYYY-MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct YearMonth {
    year: i32,
    month: u8,
}

impl YearMonth {
    pub fn new(year: i32, month: u8) -> Result<Self, BudgetError> {
        let valid = (0..=9999).contains(&year) && (1..=12).contains(&month);
        if valid {
            Ok(Self { year, month })
        } else {
            Err(BudgetError::InvalidMonth)
        }
    }

    /// Strict `YYYY-MM`: four digits, a dash, two digits.
    pub fn parse(text: &str) -> Result<Self, BudgetError> {
        let bytes = text.as_bytes();
        let shape = bytes.len() == 7
            && bytes[4] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| i == 4 || b.is_ascii_digit());
        if !shape {
            return Err(BudgetError::InvalidMonth);
        }
        let year = text[0..4].parse().map_err(|_| BudgetError::InvalidMonth)?;
        let month = text[5..7].parse().map_err(|_| BudgetError::InvalidMonth)?;
        Self::new(year, month)
    }

    /// Months since year 0: a total order that makes month arithmetic trivial.
    pub fn index(self) -> i64 {
        i64::from(self.year) * 12 + i64::from(self.month) - 1
    }

    /// The month `delta` months away (negative goes back).
    pub fn plus_months(self, delta: i32) -> Result<Self, BudgetError> {
        let index = self.index() + i64::from(delta);
        let year = i32::try_from(index.div_euclid(12)).map_err(|_| BudgetError::InvalidMonth)?;
        let month =
            u8::try_from(index.rem_euclid(12) + 1).map_err(|_| BudgetError::InvalidMonth)?;
        Self::new(year, month)
    }

    pub fn of(date: Date) -> Self {
        Self {
            year: date.year(),
            month: date.month() as u8,
        }
    }

    pub fn year(self) -> i32 {
        self.year
    }

    pub fn month(self) -> u8 {
        self.month
    }

    pub fn first_day(self) -> Date {
        self.day(1)
    }

    pub fn last_day(self) -> Date {
        let month = Month::try_from(self.month).unwrap_or(Month::January);
        self.day(days_in_month(month, self.year))
    }

    fn day(self, day: u8) -> Date {
        let month = Month::try_from(self.month).unwrap_or(Month::January);
        // Year and month are validated in `new`, so this cannot fail; the
        // fallback keeps the function total without a panic path.
        Date::from_calendar_date(self.year, month, day).unwrap_or(Date::MIN)
    }
}

impl fmt::Display for YearMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    Ok,
    /// At least [`WARN_AT_BP`] of the limit is used.
    Warning,
    /// More than the whole limit is spent.
    Over,
}

impl BudgetStatus {
    pub fn code(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Over => "over",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BudgetProgress {
    pub limit: Money,
    pub spent: Money,
    /// `limit - spent`; negative once the budget is exceeded.
    pub remaining: Money,
    /// `spent / limit` in basis points (10 000 = 100 %), rounded half away from zero.
    pub used_bp: i64,
    pub status: BudgetStatus,
}

/// `spent` is the positive amount spent in the month.
pub fn progress(limit: Money, spent: Money) -> Result<BudgetProgress, MoneyError> {
    let remaining = limit.checked_sub(&spent)?;
    let used_bp = spent
        .checked_mul_ratio(10_000, limit.minor())?
        .minor()
        .max(0);
    // The status uses exact integer arithmetic, not the rounded `used_bp`:
    // 79.999 % must not become a warning just because it displays as 80.00 %.
    let status = if spent.minor() > limit.minor() {
        BudgetStatus::Over
    } else if i128::from(spent.minor()) * 10_000
        >= i128::from(WARN_AT_BP) * i128::from(limit.minor())
    {
        BudgetStatus::Warning
    } else {
        BudgetStatus::Ok
    };
    Ok(BudgetProgress {
        limit,
        spent,
        remaining,
        used_bp,
        status,
    })
}

/// A validated budget limit for one category and month.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BudgetDraft {
    category_id: i64,
    month: YearMonth,
    limit: Money,
}

impl BudgetDraft {
    pub fn new(category_id: i64, month: YearMonth, limit: Money) -> Result<Self, BudgetError> {
        if !limit.is_positive() {
            return Err(BudgetError::NonPositiveLimit);
        }
        Ok(Self {
            category_id,
            month,
            limit,
        })
    }

    pub fn category_id(&self) -> i64 {
        self.category_id
    }

    pub fn month(&self) -> YearMonth {
        self.month
    }

    pub fn limit(&self) -> Money {
        self.limit
    }
}

// Amounts, names and notes must never reach a log through `{:?}`.
crate::redacted_debug!(BudgetProgress, BudgetDraft);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Currency;

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, Currency::IDR).unwrap()
    }

    #[test]
    fn months_parse_strictly_and_print_zero_padded() {
        let m = YearMonth::parse("2026-02").unwrap();
        assert_eq!((m.year(), m.month()), (2026, 2));
        assert_eq!(m.to_string(), "2026-02");
        for bad in [
            "",
            "2026-2",
            "2026-13",
            "2026-00",
            "26-02",
            "2026/02",
            "2026-02-01",
            "abcd-ef",
        ] {
            assert_eq!(
                YearMonth::parse(bad),
                Err(BudgetError::InvalidMonth),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn first_and_last_day_handle_month_lengths_and_leap_years() {
        let last = |s: &str| {
            let d = YearMonth::parse(s).unwrap().last_day();
            (d.year(), d.month() as u8, d.day())
        };
        assert_eq!(last("2026-01"), (2026, 1, 31));
        assert_eq!(last("2026-02"), (2026, 2, 28));
        assert_eq!(last("2028-02"), (2028, 2, 29));
        assert_eq!(last("2026-04"), (2026, 4, 30));
        let first = YearMonth::parse("2026-10").unwrap().first_day();
        assert_eq!(
            (first.year(), first.month() as u8, first.day()),
            (2026, 10, 1)
        );
    }

    #[test]
    fn month_arithmetic_crosses_year_boundaries() {
        let m = |s: &str| YearMonth::parse(s).unwrap();
        assert_eq!(m("2026-12").plus_months(1), Ok(m("2027-01")));
        assert_eq!(m("2026-01").plus_months(-1), Ok(m("2025-12")));
        assert_eq!(m("2026-10").plus_months(0), Ok(m("2026-10")));
        assert_eq!(m("2026-10").plus_months(-23), Ok(m("2024-11")));
        assert_eq!(m("0000-01").plus_months(-1), Err(BudgetError::InvalidMonth));
        assert_eq!(m("9999-12").plus_months(1), Err(BudgetError::InvalidMonth));
        assert_eq!(m("2026-03").index() - m("2025-03").index(), 12);
    }

    #[test]
    fn months_order_chronologically() {
        let a = YearMonth::parse("2025-12").unwrap();
        let b = YearMonth::parse("2026-01").unwrap();
        assert!(a < b);
        assert_eq!(YearMonth::of(b.first_day()), b);
    }

    #[test]
    fn progress_is_ok_below_the_warning_threshold() {
        let p = progress(idr(1_000_000), idr(500_000)).unwrap();
        assert_eq!((p.used_bp, p.status), (5_000, BudgetStatus::Ok));
        assert_eq!(p.remaining, idr(500_000));
    }

    #[test]
    fn progress_warns_at_eighty_percent_and_is_not_over_at_exactly_one_hundred() {
        assert_eq!(
            progress(idr(1_000), idr(799)).unwrap().status,
            BudgetStatus::Ok
        );
        assert_eq!(
            progress(idr(1_000), idr(800)).unwrap().status,
            BudgetStatus::Warning
        );
        let full = progress(idr(1_000), idr(1_000)).unwrap();
        assert_eq!((full.used_bp, full.status), (10_000, BudgetStatus::Warning));
        assert_eq!(full.remaining, idr(0));
    }

    #[test]
    fn the_warning_threshold_is_exact_not_rounded() {
        // 79.999 % rounds to 8000 bp for display but is still below the threshold.
        let just_below = progress(idr(100_000), idr(79_999)).unwrap();
        assert_eq!(
            (just_below.used_bp, just_below.status),
            (8_000, BudgetStatus::Ok)
        );
        assert_eq!(
            progress(idr(100_000), idr(80_000)).unwrap().status,
            BudgetStatus::Warning
        );
        // The same boundary at a different scale.
        assert_eq!(progress(idr(7), idr(5)).unwrap().status, BudgetStatus::Ok);
        assert_eq!(
            progress(idr(5), idr(4)).unwrap().status,
            BudgetStatus::Warning
        );
    }

    #[test]
    fn progress_is_over_beyond_the_limit_with_a_negative_remainder() {
        let p = progress(idr(1_000), idr(1_001)).unwrap();
        assert_eq!(p.status, BudgetStatus::Over);
        assert_eq!(p.remaining, idr(-1));
        assert_eq!(p.used_bp, 10_010);
    }

    #[test]
    fn progress_handles_zero_spend_and_rounding() {
        let zero = progress(idr(1_000), idr(0)).unwrap();
        assert_eq!((zero.used_bp, zero.status), (0, BudgetStatus::Ok));
        // 1 / 3 of the limit: 3333.33 bp rounds down to 3333.
        assert_eq!(progress(idr(3), idr(1)).unwrap().used_bp, 3_333);
        // 2 / 3: 6666.67 rounds up to 6667.
        assert_eq!(progress(idr(3), idr(2)).unwrap().used_bp, 6_667);
    }

    #[test]
    fn progress_survives_huge_amounts() {
        let limit = idr(crate::MAX_MINOR);
        let p = progress(limit, idr(crate::MAX_MINOR / 2)).unwrap();
        assert_eq!(p.status, BudgetStatus::Ok);
        assert!(p.used_bp > 4_900 && p.used_bp < 5_100);
    }

    #[test]
    fn a_budget_limit_must_be_positive() {
        let m = YearMonth::parse("2026-10").unwrap();
        assert!(BudgetDraft::new(1, m, idr(1)).is_ok());
        assert_eq!(
            BudgetDraft::new(1, m, idr(0)),
            Err(BudgetError::NonPositiveLimit)
        );
        assert_eq!(
            BudgetDraft::new(1, m, idr(-5)),
            Err(BudgetError::NonPositiveLimit)
        );
    }
}
