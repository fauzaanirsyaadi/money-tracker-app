//! The `Money` value type: an `i64` count of minor units plus a currency.
//!
//! No floating point is used anywhere. All arithmetic is checked, and every
//! value is kept within `-MAX_MINOR..=MAX_MINOR` so it survives a round trip
//! through JavaScript numbers if a caller ever mishandles the string encoding.

use crate::{currency::Currency, error::MoneyError};

/// Largest magnitude of a `Money` value in minor units: `2^53 - 1`.
pub const MAX_MINOR: i64 = (1_i64 << 53) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    minor: i64,
    currency: Currency,
}

impl Money {
    /// Builds a value from minor units, rejecting anything beyond `MAX_MINOR`.
    pub fn from_minor(minor: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor.unsigned_abs() > MAX_MINOR.unsigned_abs() {
            return Err(MoneyError::Overflow);
        }
        Ok(Self { minor, currency })
    }

    pub fn zero(currency: Currency) -> Self {
        Self { minor: 0, currency }
    }

    pub fn minor(&self) -> i64 {
        self.minor
    }

    pub fn currency(&self) -> Currency {
        self.currency
    }

    pub fn is_zero(&self) -> bool {
        self.minor == 0
    }

    pub fn is_negative(&self) -> bool {
        self.minor < 0
    }

    pub fn is_positive(&self) -> bool {
        self.minor > 0
    }

    fn same_currency(&self, other: &Self) -> Result<(), MoneyError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(MoneyError::CurrencyMismatch)
        }
    }

    pub fn checked_add(&self, other: &Self) -> Result<Self, MoneyError> {
        self.same_currency(other)?;
        let sum = self
            .minor
            .checked_add(other.minor)
            .ok_or(MoneyError::Overflow)?;
        Self::from_minor(sum, self.currency)
    }

    pub fn checked_sub(&self, other: &Self) -> Result<Self, MoneyError> {
        self.same_currency(other)?;
        let diff = self
            .minor
            .checked_sub(other.minor)
            .ok_or(MoneyError::Overflow)?;
        Self::from_minor(diff, self.currency)
    }

    /// Negation cannot overflow because the range is symmetric.
    pub fn negated(&self) -> Self {
        Self {
            minor: -self.minor,
            currency: self.currency,
        }
    }

    pub fn abs(&self) -> Self {
        Self {
            minor: self.minor.abs(),
            currency: self.currency,
        }
    }

    pub fn checked_mul(&self, factor: i64) -> Result<Self, MoneyError> {
        let product = self.minor.checked_mul(factor).ok_or(MoneyError::Overflow)?;
        Self::from_minor(product, self.currency)
    }

    /// Multiplies by `num / den`, rounding half away from zero.
    ///
    /// This is the only place that rounds; use it for percentages.
    pub fn checked_mul_ratio(&self, num: i64, den: i64) -> Result<Self, MoneyError> {
        if den == 0 {
            return Err(MoneyError::DivisionByZero);
        }
        let value = i128::from(self.minor) * i128::from(num);
        let den = i128::from(den);
        let mut quotient = value / den;
        let remainder = value % den;
        if remainder != 0 && remainder.unsigned_abs() * 2 >= den.unsigned_abs() {
            // Sign of the exact quotient is sign(value) * sign(den).
            quotient += (value.signum() * den.signum()).signum();
        }
        let minor = i64::try_from(quotient).map_err(|_| MoneyError::Overflow)?;
        Self::from_minor(minor, self.currency)
    }

    /// Splits the amount proportionally to `ratios` without losing a minor unit.
    ///
    /// Uses the largest-remainder method; ties go to the earlier index. The
    /// parts always add up to exactly `self`, and every part has the same sign
    /// as `self`.
    pub fn allocate(&self, ratios: &[u32]) -> Result<Vec<Self>, MoneyError> {
        let sum: u128 = ratios.iter().map(|&r| u128::from(r)).sum();
        if ratios.is_empty() || sum == 0 {
            return Err(MoneyError::InvalidRatios);
        }
        let total = u128::from(self.minor.unsigned_abs());

        let mut shares: Vec<u128> = ratios
            .iter()
            .map(|&r| total * u128::from(r) / sum)
            .collect();
        let remainders: Vec<u128> = ratios
            .iter()
            .map(|&r| total * u128::from(r) % sum)
            .collect();

        let assigned: u128 = shares.iter().sum();
        let mut order: Vec<usize> = (0..ratios.len()).collect();
        // Stable sort keeps the earlier index first among equal remainders.
        order.sort_by(|&a, &b| remainders[b].cmp(&remainders[a]));
        let leftover = usize::try_from(total - assigned).map_err(|_| MoneyError::Overflow)?;
        for &index in order.iter().take(leftover) {
            shares[index] += 1;
        }

        shares
            .into_iter()
            .map(|share| {
                let magnitude = i64::try_from(share).map_err(|_| MoneyError::Overflow)?;
                let minor = if self.minor < 0 {
                    -magnitude
                } else {
                    magnitude
                };
                Self::from_minor(minor, self.currency)
            })
            .collect()
    }

    /// Parses a canonical decimal string such as `-1234.50` (no grouping, `.`
    /// as the separator, at most `exponent` fraction digits).
    ///
    /// Locale-specific input is the UI's job; it must normalise first.
    pub fn parse_decimal(text: &str, currency: Currency) -> Result<Self, MoneyError> {
        let (negative, body) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (int_part, frac_part) = match body.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (body, None),
        };
        let exponent = usize::from(currency.exponent());
        let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());

        if int_part.is_empty() || !all_digits(int_part) {
            return Err(MoneyError::InvalidAmount);
        }
        if let Some(frac) = frac_part
            && (frac.is_empty() || frac.len() > exponent || !all_digits(frac))
        {
            return Err(MoneyError::InvalidAmount);
        }

        let mut minor: i64 = 0;
        for byte in int_part.bytes() {
            minor = minor
                .checked_mul(10)
                .and_then(|m| m.checked_add(i64::from(byte - b'0')))
                .ok_or(MoneyError::Overflow)?;
        }
        minor = minor
            .checked_mul(currency.scale())
            .ok_or(MoneyError::Overflow)?;

        let frac = frac_part.unwrap_or("");
        let mut frac_value: i64 = 0;
        for byte in frac.bytes() {
            frac_value = frac_value * 10 + i64::from(byte - b'0');
        }
        // Right-pad the fraction to `exponent` digits ("5" in USD is 50).
        for _ in frac.len()..exponent {
            frac_value *= 10;
        }
        minor = minor.checked_add(frac_value).ok_or(MoneyError::Overflow)?;

        Self::from_minor(if negative { -minor } else { minor }, currency)
    }

    /// Canonical decimal string, the inverse of [`Money::parse_decimal`].
    pub fn to_decimal_string(&self) -> String {
        let scale = self.currency.scale().unsigned_abs();
        let magnitude = self.minor.unsigned_abs();
        let sign = if self.minor < 0 { "-" } else { "" };
        let exponent = usize::from(self.currency.exponent());
        if exponent == 0 {
            return format!("{sign}{magnitude}");
        }
        let (whole, frac) = (magnitude / scale, magnitude % scale);
        format!("{sign}{whole}.{frac:0exponent$}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDR: Currency = Currency::IDR;
    const USD: Currency = Currency::USD;

    fn idr(minor: i64) -> Money {
        Money::from_minor(minor, IDR).unwrap()
    }

    fn usd(minor: i64) -> Money {
        Money::from_minor(minor, USD).unwrap()
    }

    fn minors(parts: &[Money]) -> Vec<i64> {
        parts.iter().map(Money::minor).collect()
    }

    // --- construction and range ---------------------------------------

    #[test]
    fn accepts_the_full_symmetric_range() {
        assert!(Money::from_minor(MAX_MINOR, IDR).is_ok());
        assert!(Money::from_minor(-MAX_MINOR, IDR).is_ok());
    }

    #[test]
    fn rejects_values_beyond_the_range() {
        assert_eq!(
            Money::from_minor(MAX_MINOR + 1, IDR),
            Err(MoneyError::Overflow)
        );
        assert_eq!(
            Money::from_minor(-MAX_MINOR - 1, IDR),
            Err(MoneyError::Overflow)
        );
        assert_eq!(Money::from_minor(i64::MIN, IDR), Err(MoneyError::Overflow));
        assert_eq!(Money::from_minor(i64::MAX, IDR), Err(MoneyError::Overflow));
    }

    // --- add / sub / neg -------------------------------------------------

    #[test]
    fn add_and_sub_are_exact() {
        assert_eq!(idr(1_500).checked_add(&idr(2_500)), Ok(idr(4_000)));
        assert_eq!(idr(1_500).checked_sub(&idr(2_500)), Ok(idr(-1_000)));
    }

    #[test]
    fn add_overflow_is_an_error_not_a_wrap() {
        let max = idr(MAX_MINOR);
        assert_eq!(max.checked_add(&idr(1)), Err(MoneyError::Overflow));
        assert_eq!(
            idr(-MAX_MINOR).checked_sub(&idr(1)),
            Err(MoneyError::Overflow)
        );
    }

    #[test]
    fn mixing_currencies_is_an_error() {
        assert_eq!(
            idr(1).checked_add(&usd(1)),
            Err(MoneyError::CurrencyMismatch)
        );
        assert_eq!(
            idr(1).checked_sub(&usd(1)),
            Err(MoneyError::CurrencyMismatch)
        );
    }

    #[test]
    fn negation_and_abs_stay_in_range() {
        assert_eq!(idr(MAX_MINOR).negated(), idr(-MAX_MINOR));
        assert_eq!(idr(-5).abs(), idr(5));
        assert_eq!(idr(5).abs(), idr(5));
    }

    #[test]
    fn mul_is_checked() {
        assert_eq!(idr(1_000).checked_mul(3), Ok(idr(3_000)));
        assert_eq!(idr(1_000).checked_mul(-2), Ok(idr(-2_000)));
        assert_eq!(idr(MAX_MINOR).checked_mul(2), Err(MoneyError::Overflow));
        assert_eq!(
            idr(MAX_MINOR).checked_mul(i64::MAX),
            Err(MoneyError::Overflow)
        );
    }

    // --- ratio / rounding ------------------------------------------------

    #[test]
    fn ratio_rounds_half_away_from_zero() {
        // 5 * 1/2 = 2.5 -> 3 ; -5 * 1/2 = -2.5 -> -3
        assert_eq!(idr(5).checked_mul_ratio(1, 2), Ok(idr(3)));
        assert_eq!(idr(-5).checked_mul_ratio(1, 2), Ok(idr(-3)));
        // negative denominator flips the sign of the exact quotient
        assert_eq!(idr(5).checked_mul_ratio(1, -2), Ok(idr(-3)));
        // below half rounds down
        assert_eq!(idr(10).checked_mul_ratio(1, 3), Ok(idr(3)));
        assert_eq!(idr(-10).checked_mul_ratio(1, 3), Ok(idr(-3)));
        // above half rounds up
        assert_eq!(idr(20).checked_mul_ratio(1, 3), Ok(idr(7)));
    }

    #[test]
    fn ratio_handles_percentages_and_zero() {
        assert_eq!(idr(1_000_000).checked_mul_ratio(15, 100), Ok(idr(150_000)));
        assert_eq!(idr(0).checked_mul_ratio(7, 3), Ok(idr(0)));
        assert_eq!(
            idr(1).checked_mul_ratio(1, 0),
            Err(MoneyError::DivisionByZero)
        );
    }

    #[test]
    fn ratio_uses_a_wide_intermediate() {
        // MAX * MAX does not fit i64, but the exact result (MAX) does.
        assert_eq!(
            idr(MAX_MINOR).checked_mul_ratio(MAX_MINOR, MAX_MINOR),
            Ok(idr(MAX_MINOR))
        );
        assert_eq!(
            idr(MAX_MINOR).checked_mul_ratio(2, 1),
            Err(MoneyError::Overflow)
        );
    }

    // --- allocate --------------------------------------------------------

    #[test]
    fn allocate_never_loses_a_minor_unit() {
        let parts = idr(100).allocate(&[1, 1, 1]).unwrap();
        assert_eq!(minors(&parts), vec![34, 33, 33]);
        assert_eq!(parts.iter().map(Money::minor).sum::<i64>(), 100);
    }

    #[test]
    fn allocate_follows_the_ratios() {
        let parts = idr(1_000).allocate(&[70, 20, 10]).unwrap();
        assert_eq!(minors(&parts), vec![700, 200, 100]);
        let parts = usd(5).allocate(&[3, 7]).unwrap();
        assert_eq!(minors(&parts), vec![2, 3]); // 1.5 and 3.5 -> 2 and 3
    }

    #[test]
    fn allocate_uses_largest_remainder_with_stable_ties() {
        // 10 split 1:1:1 -> 3.33 each; the leftover unit goes to the first.
        assert_eq!(
            minors(&idr(10).allocate(&[1, 1, 1]).unwrap()),
            vec![4, 3, 3]
        );
        // 2 split 1:1:1 -> remainders tie; first two get a unit.
        assert_eq!(minors(&idr(2).allocate(&[1, 1, 1]).unwrap()), vec![1, 1, 0]);
        // Larger remainder wins even when it is later in the list.
        assert_eq!(
            minors(&idr(10).allocate(&[1, 6, 3]).unwrap()),
            vec![1, 6, 3]
        );
        assert_eq!(minors(&idr(7).allocate(&[1, 1, 5]).unwrap()), vec![1, 1, 5]);
        assert_eq!(minors(&idr(8).allocate(&[3, 3, 2]).unwrap()), vec![3, 3, 2]);
    }

    #[test]
    fn allocate_negative_amounts_keep_the_sign() {
        let parts = idr(-100).allocate(&[1, 1, 1]).unwrap();
        assert_eq!(minors(&parts), vec![-34, -33, -33]);
    }

    #[test]
    fn allocate_handles_zero_ratios_and_zero_amount() {
        assert_eq!(minors(&idr(100).allocate(&[0, 1]).unwrap()), vec![0, 100]);
        assert_eq!(minors(&idr(0).allocate(&[1, 2]).unwrap()), vec![0, 0]);
    }

    #[test]
    fn allocate_rejects_bad_ratios() {
        assert_eq!(idr(100).allocate(&[]), Err(MoneyError::InvalidRatios));
        assert_eq!(idr(100).allocate(&[0, 0]), Err(MoneyError::InvalidRatios));
    }

    #[test]
    fn allocate_handles_huge_ratios_and_amounts() {
        let parts = idr(MAX_MINOR).allocate(&[u32::MAX, u32::MAX, 1]).unwrap();
        assert_eq!(parts.iter().map(Money::minor).sum::<i64>(), MAX_MINOR);
        assert!(parts.iter().all(|p| p.minor() >= 0));
    }

    // --- parse / format ----------------------------------------------------

    #[test]
    fn parse_idr_has_no_decimals() {
        assert_eq!(Money::parse_decimal("12500", IDR), Ok(idr(12_500)));
        assert_eq!(Money::parse_decimal("-12500", IDR), Ok(idr(-12_500)));
        assert_eq!(Money::parse_decimal("0", IDR), Ok(idr(0)));
        assert_eq!(
            Money::parse_decimal("12500.5", IDR),
            Err(MoneyError::InvalidAmount)
        );
        assert_eq!(
            Money::parse_decimal("12500.", IDR),
            Err(MoneyError::InvalidAmount)
        );
    }

    #[test]
    fn parse_pads_and_limits_fraction_digits() {
        assert_eq!(Money::parse_decimal("12.5", USD), Ok(usd(1_250)));
        assert_eq!(Money::parse_decimal("12.05", USD), Ok(usd(1_205)));
        assert_eq!(Money::parse_decimal("12", USD), Ok(usd(1_200)));
        assert_eq!(Money::parse_decimal("-0.99", USD), Ok(usd(-99)));
        assert_eq!(
            Money::parse_decimal("12.345", USD),
            Err(MoneyError::InvalidAmount)
        );
        let kwd = Currency::KWD;
        assert_eq!(
            Money::parse_decimal("1.5", kwd),
            Ok(Money::from_minor(1_500, kwd).unwrap())
        );
    }

    #[test]
    fn parse_rejects_malformed_input() {
        for bad in [
            "", "-", ".", ".5", "1..2", "1.2.3", "abc", "1e5", " 1", "1 ", "+1", "--1", "1,5", "١٢",
        ] {
            assert_eq!(
                Money::parse_decimal(bad, USD),
                Err(MoneyError::InvalidAmount),
                "input {bad:?}"
            );
        }
    }

    #[test]
    fn parse_detects_overflow_without_panicking() {
        assert_eq!(
            Money::parse_decimal("9007199254740991", IDR),
            Ok(idr(MAX_MINOR))
        );
        assert_eq!(
            Money::parse_decimal("9007199254740992", IDR),
            Err(MoneyError::Overflow)
        );
        assert_eq!(
            Money::parse_decimal("99999999999999999999999999999", IDR),
            Err(MoneyError::Overflow)
        );
        assert_eq!(
            Money::parse_decimal("99999999999999999999.99", USD),
            Err(MoneyError::Overflow)
        );
    }

    #[test]
    fn format_is_canonical() {
        assert_eq!(idr(12_500).to_decimal_string(), "12500");
        assert_eq!(idr(-12_500).to_decimal_string(), "-12500");
        assert_eq!(idr(0).to_decimal_string(), "0");
        assert_eq!(usd(1_205).to_decimal_string(), "12.05");
        assert_eq!(usd(5).to_decimal_string(), "0.05");
        assert_eq!(usd(-5).to_decimal_string(), "-0.05");
        assert_eq!(usd(0).to_decimal_string(), "0.00");
        assert_eq!(
            Money::from_minor(-1, Currency::KWD)
                .unwrap()
                .to_decimal_string(),
            "-0.001"
        );
    }

    #[test]
    fn parse_and_format_round_trip_at_the_edges() {
        for minor in [0, 1, -1, 99, 100, 101, MAX_MINOR, -MAX_MINOR] {
            for currency in [IDR, USD, Currency::KWD] {
                let money = Money::from_minor(minor, currency).unwrap();
                let text = money.to_decimal_string();
                assert_eq!(Money::parse_decimal(&text, currency), Ok(money), "{text}");
            }
        }
    }
}
