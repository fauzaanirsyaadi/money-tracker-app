use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors carry no key material or payloads.
#[derive(Debug)]
pub enum Error {
    /// No Android Keystore on this platform.
    Unsupported,
    /// The plugin returned malformed data.
    BadResponse,
    /// The Keystore failed in a way that may pass (service busy, device
    /// locked). Nothing should be created or deleted because of it.
    Keystore,
    /// The wrapping key is permanently unusable: the alias is gone, the key was
    /// invalidated, or the blob no longer authenticates under it.
    KeyGone,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "keystore unsupported on this platform",
            Self::BadResponse => "malformed keystore response",
            Self::Keystore => "keystore operation failed",
            Self::KeyGone => "the keystore key is gone or unusable",
        })
    }
}

impl std::error::Error for Error {}

/// The Kotlin side rejects with the code `key_gone` **only** for an explicit
/// permanent signal. Any other code, no code, or a failure of the bridge itself
/// is transient: unknown errors never lead to recovery.
#[cfg_attr(
    not(mobile),
    allow(dead_code, reason = "used by the Android bridge and by tests")
)]
pub(crate) fn classify_rejection(code: Option<&str>) -> Error {
    match code {
        Some("key_gone") => Error::KeyGone,
        _ => Error::Keystore,
    }
}

#[cfg(mobile)]
impl From<tauri::plugin::mobile::PluginInvokeError> for Error {
    fn from(err: tauri::plugin::mobile::PluginInvokeError) -> Self {
        use tauri::plugin::mobile::PluginInvokeError::InvokeRejected;
        match err {
            InvokeRejected(response) => classify_rejection(response.code.as_deref()),
            _ => Self::Keystore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_explicit_permanent_code_means_the_key_is_gone() {
        assert!(matches!(
            classify_rejection(Some("key_gone")),
            Error::KeyGone
        ));
    }

    #[test]
    fn every_other_rejection_is_transient() {
        for code in [
            None,
            Some("keystore"),
            Some("path"),
            Some(""),
            Some("KEY_GONE"),
            Some("key_gone "),
            Some("unknown"),
        ] {
            assert!(
                matches!(classify_rejection(code), Error::Keystore),
                "{code:?}"
            );
        }
    }
}
