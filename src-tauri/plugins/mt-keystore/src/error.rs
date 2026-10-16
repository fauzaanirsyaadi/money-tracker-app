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

#[cfg(mobile)]
impl From<tauri::plugin::mobile::PluginInvokeError> for Error {
    fn from(err: tauri::plugin::mobile::PluginInvokeError) -> Self {
        use tauri::plugin::mobile::PluginInvokeError::InvokeRejected;
        match err {
            // Kotlin rejects with the code `key_gone` only for permanent loss.
            InvokeRejected(response) if response.code.as_deref() == Some("key_gone") => {
                Self::KeyGone
            }
            _ => Self::Keystore,
        }
    }
}
