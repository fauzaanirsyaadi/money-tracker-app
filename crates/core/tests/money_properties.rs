//! Property tests for `Money` (docs/ARCHITECTURE.md section 4).
//!
//! Money is the one type whose bugs lose people's money silently, so its laws are
//! checked on random inputs, including the extremes, not only on hand-picked cases.

#![allow(clippy::unwrap_used, reason = "a failed unwrap is a failed test")]

use mt_core::{Currency, MAX_MINOR, Money, MoneyError};
use proptest::prelude::*;

const CURRENCIES: [Currency; 6] = [
    Currency::IDR,
    Currency::USD,
    Currency::EUR,
    Currency::JPY,
    Currency::KRW,
    Currency::SGD,
];

fn currency() -> impl Strategy<Value = Currency> {
    prop::sample::select(CURRENCIES.to_vec())
}

/// Any representable amount, with the extremes over-represented.
fn minor() -> impl Strategy<Value = i64> {
    prop_oneof![
        3 => -MAX_MINOR..=MAX_MINOR,
        1 => Just(MAX_MINOR),
        1 => Just(-MAX_MINOR),
        1 => Just(0),
        1 => -1000_i64..=1000,
    ]
}

fn money() -> impl Strategy<Value = Money> {
    (minor(), currency()).prop_map(|(m, c)| Money::from_minor(m, c).unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // --- construction ---------------------------------------------------------

    #[test]
    fn from_minor_accepts_exactly_the_symmetric_range(m in any::<i64>(), c in currency()) {
        let built = Money::from_minor(m, c);
        if m.unsigned_abs() <= MAX_MINOR.unsigned_abs() {
            prop_assert_eq!(built.unwrap().minor(), m);
        } else {
            prop_assert_eq!(built, Err(MoneyError::Overflow));
        }
    }

    // --- addition and subtraction ---------------------------------------------

    #[test]
    fn adding_then_subtracting_gives_the_start_back(a in minor(), b in minor(), c in currency()) {
        let (x, y) = (Money::from_minor(a, c).unwrap(), Money::from_minor(b, c).unwrap());
        match x.checked_add(&y) {
            Ok(sum) => {
                prop_assert_eq!(sum.checked_sub(&y).unwrap(), x);
                prop_assert_eq!(sum, y.checked_add(&x).unwrap(), "addition commutes");
            }
            Err(error) => {
                // Only the range can refuse an addition of two valid amounts.
                prop_assert_eq!(error, MoneyError::Overflow);
                prop_assert!(i128::from(a) + i128::from(b) > i128::from(MAX_MINOR)
                    || i128::from(a) + i128::from(b) < -i128::from(MAX_MINOR));
            }
        }
    }

    #[test]
    fn a_value_minus_itself_is_zero_and_negation_is_an_involution(x in money()) {
        prop_assert!(x.checked_sub(&x).unwrap().is_zero());
        prop_assert_eq!(x.negated().negated(), x);
        prop_assert_eq!(x.abs().minor(), x.minor().abs());
        prop_assert!(x.checked_add(&x.negated()).unwrap().is_zero());
    }

    #[test]
    fn different_currencies_never_mix(a in minor(), b in minor()) {
        let (x, y) = (Money::from_minor(a, Currency::IDR).unwrap(), Money::from_minor(b, Currency::USD).unwrap());
        prop_assert_eq!(x.checked_add(&y), Err(MoneyError::CurrencyMismatch));
        prop_assert_eq!(x.checked_sub(&y), Err(MoneyError::CurrencyMismatch));
    }

    // --- allocate ---------------------------------------------------------------

    #[test]
    fn allocate_never_loses_or_invents_a_minor_unit(
        x in money(),
        ratios in prop::collection::vec(0_u32..=u32::MAX, 1..12),
    ) {
        match x.allocate(&ratios) {
            Err(error) => {
                prop_assert_eq!(error, MoneyError::InvalidRatios);
                prop_assert!(ratios.iter().all(|r| *r == 0));
            }
            Ok(parts) => {
                prop_assert_eq!(parts.len(), ratios.len());
                let total: i128 = parts.iter().map(|p| i128::from(p.minor())).sum();
                prop_assert_eq!(total, i128::from(x.minor()), "the parts add up to the whole");
                let sum: u128 = ratios.iter().map(|r| u128::from(*r)).sum();
                for (part, ratio) in parts.iter().zip(&ratios) {
                    prop_assert_eq!(part.currency(), x.currency());
                    // Every part has the sign of the whole, or is zero.
                    prop_assert!(part.minor() == 0 || (part.minor() < 0) == (x.minor() < 0));
                    // A zero ratio gets nothing.
                    if *ratio == 0 {
                        prop_assert!(part.is_zero());
                    }
                    // Within one unit of the exact proportional share.
                    let exact_times_sum = u128::from(x.minor().unsigned_abs()) * u128::from(*ratio);
                    let got_times_sum = u128::from(part.minor().unsigned_abs()) * sum;
                    prop_assert!(got_times_sum.abs_diff(exact_times_sum) < sum,
                        "a share is within one unit of the exact proportion");
                }
            }
        }
    }

    #[test]
    fn allocating_the_negative_mirrors_allocating_the_positive(
        x in money(),
        ratios in prop::collection::vec(1_u32..=1000, 1..8),
    ) {
        let positive = x.abs().allocate(&ratios).unwrap();
        let negative = x.abs().negated().allocate(&ratios).unwrap();
        for (p, n) in positive.iter().zip(&negative) {
            prop_assert_eq!(p.minor(), -n.minor());
        }
    }

    // --- multiplication and rounding ---------------------------------------------

    #[test]
    fn mul_ratio_rounds_to_the_nearest_with_ties_away_from_zero(
        x in money(),
        num in -1_000_000_i64..=1_000_000,
        den in prop_oneof![-1_000_000_i64..=-1, 1_i64..=1_000_000],
    ) {
        match x.checked_mul_ratio(num, den) {
            Err(error) => prop_assert_eq!(error, MoneyError::Overflow),
            Ok(result) => {
                let exact_numerator = i128::from(x.minor()) * i128::from(num); // exact = this / den
                let error = i128::from(result.minor()) * i128::from(den) - exact_numerator;
                prop_assert!(2 * error.abs() <= i128::from(den).abs(), "nearest");
                if 2 * error.abs() == i128::from(den).abs() {
                    // A tie: the larger magnitude wins.
                    prop_assert!((i128::from(result.minor()) * i128::from(den)).abs() > exact_numerator.abs());
                }
            }
        }
    }

    /// Random inputs almost never land exactly on a tie, so ties are built:
    /// (2m+1)*k / (2k) is exactly m + 1/2.
    #[test]
    fn exact_halves_round_away_from_zero(
        m in 0_i64..=1_000_000,
        k in 1_i64..=1000,
        negative_amount in any::<bool>(),
        negative_den in any::<bool>(),
        c in currency(),
    ) {
        let magnitude = (2 * m + 1) * k;
        let amount = Money::from_minor(if negative_amount { -magnitude } else { magnitude }, c).unwrap();
        let den = if negative_den { -2 * k } else { 2 * k };
        let result = amount.checked_mul_ratio(1, den).unwrap();
        // The exact answer is +-(m + 1/2): the larger magnitude, m + 1, wins.
        prop_assert_eq!(result.minor().abs(), m + 1);
        prop_assert_eq!(result.minor() < 0, negative_amount != negative_den);
    }

    #[test]
    fn dividing_by_zero_is_an_error_never_a_panic(x in money(), num in any::<i64>()) {
        prop_assert_eq!(x.checked_mul_ratio(num, 0), Err(MoneyError::DivisionByZero));
    }

    #[test]
    fn mul_never_panics_and_respects_the_range(x in money(), factor in any::<i64>()) {
        match x.checked_mul(factor) {
            Ok(product) => prop_assert_eq!(i128::from(product.minor()), i128::from(x.minor()) * i128::from(factor)),
            Err(error) => prop_assert_eq!(error, MoneyError::Overflow),
        }
    }

    // --- text -------------------------------------------------------------------------

    #[test]
    fn formatting_then_parsing_returns_the_same_money(x in money()) {
        let text = x.to_decimal_string();
        prop_assert_eq!(Money::parse_decimal(&text, x.currency()).unwrap(), x, "{}", text);
    }

    #[test]
    fn parsing_arbitrary_text_never_panics_and_what_parses_is_canonical(
        text in "\\PC{0,30}",
        c in currency(),
    ) {
        if let Ok(parsed) = Money::parse_decimal(&text, c) {
            prop_assert_eq!(Money::parse_decimal(&parsed.to_decimal_string(), c).unwrap(), parsed);
        }
    }

    #[test]
    fn parsing_digit_soup_never_panics_even_when_enormous(
        text in "-?[0-9]{0,40}(\\.[0-9]{0,6})?",
        c in currency(),
    ) {
        let _ = Money::parse_decimal(&text, c);
    }
}
