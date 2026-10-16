//! ISO 4217 currency code plus the number of minor-unit decimals.

use crate::error::MoneyError;

/// Largest supported exponent (CLF uses 4).
const MAX_EXPONENT: u8 = 4;

/// A currency: three upper-case ASCII letters and its minor-unit exponent.
///
/// `exponent` is the number of decimals: IDR = 0, USD = 2, KWD = 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Currency {
    code: [u8; 3],
    exponent: u8,
}

impl Currency {
    pub const IDR: Self = Self::known(*b"IDR", 0);
    pub const USD: Self = Self::known(*b"USD", 2);
    pub const EUR: Self = Self::known(*b"EUR", 2);
    pub const GBP: Self = Self::known(*b"GBP", 2);
    pub const SGD: Self = Self::known(*b"SGD", 2);
    pub const MYR: Self = Self::known(*b"MYR", 2);
    pub const AUD: Self = Self::known(*b"AUD", 2);
    pub const CNY: Self = Self::known(*b"CNY", 2);
    pub const JPY: Self = Self::known(*b"JPY", 0);
    pub const KRW: Self = Self::known(*b"KRW", 0);
    pub const KWD: Self = Self::known(*b"KWD", 3);

    const ALL: [Self; 11] = [
        Self::IDR,
        Self::USD,
        Self::EUR,
        Self::GBP,
        Self::SGD,
        Self::MYR,
        Self::AUD,
        Self::CNY,
        Self::JPY,
        Self::KRW,
        Self::KWD,
    ];

    const fn known(code: [u8; 3], exponent: u8) -> Self {
        Self { code, exponent }
    }

    /// Builds a currency from a code and its exponent.
    ///
    /// A code in the built-in table has exactly one correct exponent, and any
    /// other is refused: accepting `IDR` with 2 decimals would read `1000` as
    /// `10.00` and silently scale every amount by 100. A code that is not in
    /// the table is accepted with the exponent the caller states (0 to 4).
    pub fn new(code: &str, exponent: u8) -> Result<Self, MoneyError> {
        let bytes = code.as_bytes();
        if bytes.len() != 3 || !bytes.iter().all(u8::is_ascii_uppercase) || exponent > MAX_EXPONENT
        {
            return Err(MoneyError::InvalidCurrency);
        }
        let candidate = Self {
            code: [bytes[0], bytes[1], bytes[2]],
            exponent,
        };
        match Self::ALL.iter().find(|known| known.code == candidate.code) {
            Some(known) if known.exponent != exponent => Err(MoneyError::InvalidCurrency),
            _ => Ok(candidate),
        }
    }

    /// Looks a currency up in the built-in table by ISO code.
    pub fn from_code(code: &str) -> Result<Self, MoneyError> {
        Self::ALL
            .into_iter()
            .find(|c| c.code().eq(code))
            .ok_or(MoneyError::InvalidCurrency)
    }

    pub fn code(&self) -> &str {
        // The constructor only admits ASCII upper-case letters.
        std::str::from_utf8(&self.code).unwrap_or("???")
    }

    pub fn exponent(&self) -> u8 {
        self.exponent
    }

    /// `10^exponent`: how many minor units make one major unit.
    pub(crate) fn scale(&self) -> i64 {
        10_i64.pow(u32::from(self.exponent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponents_match_iso() {
        assert_eq!(Currency::IDR.exponent(), 0);
        assert_eq!(Currency::USD.exponent(), 2);
        assert_eq!(Currency::JPY.exponent(), 0);
        assert_eq!(Currency::KWD.exponent(), 3);
    }

    #[test]
    fn code_round_trip() {
        assert_eq!(Currency::IDR.code(), "IDR");
        assert_eq!(Currency::from_code("USD"), Ok(Currency::USD));
    }

    #[test]
    fn a_known_code_only_accepts_its_own_exponent() {
        for known in Currency::ALL {
            assert_eq!(Currency::new(known.code(), known.exponent()), Ok(known));
            for wrong in (0..=MAX_EXPONENT).filter(|e| *e != known.exponent()) {
                assert_eq!(
                    Currency::new(known.code(), wrong),
                    Err(MoneyError::InvalidCurrency),
                    "{} with {wrong} decimals",
                    known.code()
                );
            }
        }
        // The cases that matter most for this app and its reviewers.
        assert_eq!(Currency::new("IDR", 0), Ok(Currency::IDR));
        assert_eq!(Currency::new("IDR", 2), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("USD", 2), Ok(Currency::USD));
        assert_eq!(Currency::new("USD", 0), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("KWD", 3), Ok(Currency::KWD));
        assert_eq!(Currency::new("KWD", 2), Err(MoneyError::InvalidCurrency));
    }

    #[test]
    fn an_unlisted_code_takes_the_stated_exponent() {
        // Bahraini dinar uses three decimals and is not in the table.
        let bhd = Currency::new("BHD", 3).unwrap();
        assert_eq!((bhd.code(), bhd.exponent(), bhd.scale()), ("BHD", 3, 1000));
        assert_eq!(Currency::from_code("BHD"), Err(MoneyError::InvalidCurrency));
    }

    #[test]
    fn rejects_unknown_and_malformed() {
        assert_eq!(Currency::from_code("XXX"), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("idr", 0), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("ID", 0), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("IDRR", 0), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("IDR", 5), Err(MoneyError::InvalidCurrency));
        assert_eq!(Currency::new("XYZ", 4).map(|c| c.scale()), Ok(10_000));
        assert_eq!(Currency::new("XYZ", 5), Err(MoneyError::InvalidCurrency));
    }
}
