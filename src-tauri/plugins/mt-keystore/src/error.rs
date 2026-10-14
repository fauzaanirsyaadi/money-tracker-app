use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors carry no key material or payloads.
#[derive(Debug)]
pub enum Error {
    /// No Android Keystore on this platform.
    Unsupported,
    /// The plugin returned malformed data.
    BadResponse,
    /// The Keystore refused the operation (missing key, auth failure, tampered blob).
    Keystore,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "keystore unsupported on this platform",
            Self::BadResponse => "malformed keystore response",
            Self::Keystore => "keystore operation failed",
        })
    }
}

impl std::error::Error for Error {}

#[cfg(mobile)]
impl From<tauri::plugin::mobile::PluginInvokeError> for Error {
    fn from(_: tauri::plugin::mobile::PluginInvokeError) -> Self {
        Self::Keystore
    }
}
