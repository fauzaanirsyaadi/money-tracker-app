//! Lower-case hex, the wire format between the native side and Rust.

pub fn encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// `None` for odd length or any character that is not a hex digit.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high << 4 | low).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_malformed_text() {
        let bytes = [0u8, 1, 15, 16, 127, 128, 255];
        assert_eq!(encode(&bytes), "00010f107f80ff");
        assert_eq!(decode("00010F107F80FF").as_deref(), Some(&bytes[..]));
        assert_eq!(decode("").as_deref(), Some(&[][..]));
        for bad in ["0", "abc", "zz", "0g", "+1", " 1", "é1"] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }
}
